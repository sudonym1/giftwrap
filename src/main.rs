use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nix::unistd::{getgid, getuid};
use serde::Serialize;

use giftwrap::cli::{self, CacheCommands, Commands, RunArgs};
use giftwrap::config;
use giftwrap::context_hash::{self, ContextHashResult};
use giftwrap::discovery;
use giftwrap::errors::GiftwrapError;
use giftwrap::log::Logger;
use giftwrap::oci;
use giftwrap::rootfs_builder;
use giftwrap::runtime;
use giftwrap::runtime::bwrap::{self, RunSpec};
use giftwrap::sqfs_cache::{self, CacheMetadata, CachePaths, GcOptions, PullPolicy};
use giftwrap::tooling::{self, ProbedTools};

fn main() {
    let cli = cli::parse();

    match dispatch(cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("Error: {err}");
            if let Some(hint) = err.hint() {
                eprintln!("Hint: {hint}");
            }
            std::process::exit(err.exit_code());
        }
    }
}

fn dispatch(cli: giftwrap::cli::Cli) -> Result<i32, GiftwrapError> {
    match cli.command {
        Commands::Run(run_args) => handle_run(run_args),
        Commands::PrintConfig => handle_print_config(),
        Commands::Cache(cache_args) => match cache_args.command {
            CacheCommands::Gc(gc_args) => handle_cache_gc(gc_args),
        },
        Commands::Version => {
            println!("{}", giftwrap::VERSION);
            Ok(0)
        }
    }
}

fn handle_print_config() -> Result<i32, GiftwrapError> {
    let cwd = std::env::current_dir().map_err(|err| {
        GiftwrapError::runtime(format!("failed to read current directory: {err}"))
    })?;
    let discovered = discovery::discover(&cwd)?;
    let cfg = config::load(&discovered.config_path)?;

    #[derive(Serialize)]
    struct PrintConfigOutput {
        build_root: PathBuf,
        config_path: PathBuf,
        image: String,
        setup_script: PathBuf,
        resolved_setup_script: PathBuf,
        env: BTreeMap<String, String>,
    }

    let output = PrintConfigOutput {
        build_root: discovered.build_root.clone(),
        config_path: discovered.config_path,
        image: cfg.image.clone(),
        setup_script: cfg.setup_script.clone(),
        resolved_setup_script: cfg.resolve_setup_script(&discovered.build_root),
        env: cfg.env.clone(),
    };

    let json = serde_json::to_string_pretty(&output)
        .map_err(|err| GiftwrapError::runtime(format!("failed to encode config output: {err}")))?;
    println!("{json}");
    Ok(0)
}

fn handle_cache_gc(args: giftwrap::cli::CacheGcArgs) -> Result<i32, GiftwrapError> {
    let cache_root = args
        .cache_dir
        .unwrap_or_else(sqfs_cache::default_cache_root);
    let report = sqfs_cache::gc(
        &cache_root,
        &GcOptions {
            dry_run: args.print,
            max_age_days: args.max_age_days,
        },
    )?;

    for message in &report.messages {
        if args.print {
            println!("Would {message}");
        } else {
            println!("{message}");
        }
    }

    if args.print {
        println!("Dry run: {} action(s)", report.messages.len());
    } else {
        println!("Removed {} path(s)", report.removed);
    }

    Ok(0)
}

fn handle_run(args: RunArgs) -> Result<i32, GiftwrapError> {
    let logger = Logger::new(args.verbose);
    logger.event("probing required tools");
    let tools = tooling::probe_required(&logger)?;

    let cwd = std::env::current_dir().map_err(|err| {
        GiftwrapError::runtime(format!("failed to read current directory: {err}"))
    })?;

    logger.event(format!("discovering config from {}", cwd.display()));
    let discovered = discovery::discover(&cwd)?;
    logger.event(format!(
        "config discovery result: {}",
        discovered.config_path.display()
    ));

    let cfg = config::load(&discovered.config_path)?;
    let context = {
        let _timer = logger.phase("context-hash");
        context_hash::compute(&discovered.build_root, &cfg)?
    };
    logger.event(format!("computed ctx_sha: {}", context.ctx_sha));
    write_context_marker(&discovered.build_root, &context.ctx_sha)?;

    let cache_root = args
        .cache_dir
        .clone()
        .unwrap_or_else(sqfs_cache::default_cache_root);
    let paths = sqfs_cache::resolve_paths(&cache_root, &context.ctx_sha);
    sqfs_cache::ensure_layout(&paths)?;

    let run_spec = build_run_spec(
        &discovered.build_root,
        &context.ctx_sha,
        &cwd,
        &paths.mountpoint,
        &cfg.env,
        args.command.clone(),
    );

    if args.print {
        print_run_plan(&discovered.build_root, &context, &paths, &run_spec);
        return Ok(0);
    }

    let pull_policy = args.pull.as_pull_policy();
    let should_reset_overlay =
        args.reset_overlay || args.rebuild || pull_policy == PullPolicy::Always;
    if should_reset_overlay {
        runtime::reset_overlay(&run_spec, &logger)?;
    }

    check_userns_support()?;

    let mut cache_hit = false;

    if !args.rebuild {
        cache_hit = sqfs_cache::is_valid(&paths, pull_policy)?;
        if cache_hit {
            logger.event("cache hit: reusing existing artifact");
        } else {
            logger.event("cache miss: artifact missing or invalid");
        }
    } else {
        logger.event("cache bypassed due to --rebuild");
    }

    if !cache_hit || args.rebuild {
        let timeout = Duration::from_secs(10 * 60);
        sqfs_cache::with_lock(&paths, timeout, || {
            if !args.rebuild && sqfs_cache::is_valid(&paths, pull_policy)? {
                logger.event("cache hit after lock acquisition");
                return Ok(());
            }

            build_cache_artifact(
                &paths,
                &cfg,
                &context,
                &discovered.build_root,
                pull_policy,
                &tools,
                &cache_root,
                &logger,
            )
        })?;
    }

    if args.command.is_empty() {
        return Ok(0);
    }

    runtime::run_with_mount(
        &paths.sqfs,
        &paths.mountpoint,
        &run_spec,
        &tools.unmount_tool,
        &logger,
    )
}

fn build_run_spec(
    build_root: &Path,
    ctx_sha: &str,
    cwd: &Path,
    mountpoint: &Path,
    env: &BTreeMap<String, String>,
    argv: Vec<String>,
) -> RunSpec {
    let overlay_root = build_root.join(".giftwrap").join(ctx_sha);

    RunSpec {
        host_uid: getuid().as_raw(),
        host_gid: getgid().as_raw(),
        build_root: build_root.to_path_buf(),
        workdir: cwd.to_path_buf(),
        mountpoint: mountpoint.to_path_buf(),
        overlay_root: overlay_root.clone(),
        overlay_upper: overlay_root.join("upper"),
        overlay_work: overlay_root.join("work"),
        env: runtime::merged_env_from_host(env),
        argv,
    }
}

fn write_context_marker(build_root: &Path, ctx_sha: &str) -> Result<(), GiftwrapError> {
    let state_root = build_root.join(".giftwrap");
    fs::create_dir_all(&state_root).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to create state directory {}: {err}",
            state_root.display()
        ))
    })?;

    let context_path = state_root.join("context");
    let tmp_path = state_root.join(format!("context.tmp.{}", std::process::id()));
    let payload = format!("{ctx_sha}\n");

    fs::write(&tmp_path, payload).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to write context marker {}: {err}",
            tmp_path.display()
        ))
    })?;

    if let Err(err) = fs::rename(&tmp_path, &context_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(GiftwrapError::runtime(format!(
            "failed to update context marker {}: {err}",
            context_path.display()
        )));
    }

    Ok(())
}

fn print_run_plan(
    build_root: &Path,
    context: &ContextHashResult,
    paths: &CachePaths,
    run_spec: &RunSpec,
) {
    let runtime_argv = bwrap::build_argv(run_spec);
    println!("Build root: {}", build_root.display());
    println!("Context hash: {}", context.ctx_sha);
    println!("Cache sqfs: {}", paths.sqfs.display());
    println!("Cache meta: {}", paths.meta.display());
    println!("Cache lock: {}", paths.lock.display());
    println!("Cache mountpoint: {}", paths.mountpoint.display());
    println!("Setup command: /bin/sh /tmp/giftwrap-setup.sh");
    println!("Runtime bwrap argv: bwrap {}", runtime_argv.join(" "));
}

#[allow(clippy::too_many_arguments)]
fn build_cache_artifact(
    paths: &CachePaths,
    cfg: &config::Config,
    context: &ContextHashResult,
    build_root: &Path,
    pull_policy: PullPolicy,
    tools: &ProbedTools,
    cache_root: &Path,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    let work_dir = paths.work_root.join(format!(
        "{}-{}-{}",
        context.ctx_sha,
        std::process::id(),
        unix_millis()
    ));
    fs::create_dir_all(&work_dir).map_err(|err| {
        GiftwrapError::build(format!(
            "failed to create work directory {}: {err}",
            work_dir.display()
        ))
    })?;

    let build_result = (|| {
        let layout_dir = work_dir.join("oci");
        let bundle_dir = work_dir.join("bundle");

        {
            let _timer = logger.phase("pull");
            oci::pull_to_layout(&cfg.image, &layout_dir, logger)?;
        }
        let image_digest = {
            let _timer = logger.phase("inspect");
            oci::inspect_digest(&cfg.image, logger)?
        };
        let rootfs = {
            let _timer = logger.phase("unpack");
            rootfs_builder::unpack(&layout_dir, &bundle_dir, logger)?
        };
        {
            let _timer = logger.phase("setup");
            rootfs_builder::run_setup(
                &rootfs,
                cfg,
                &context.ctx_sha,
                build_root,
                cache_root,
                logger,
            )?;
        }

        let sqfs_tmp = paths.sqfs.with_extension("sqfs.tmp");
        if sqfs_tmp.exists() {
            fs::remove_file(&sqfs_tmp).map_err(|err| {
                GiftwrapError::build(format!(
                    "failed to clean old temp artifact {}: {err}",
                    sqfs_tmp.display()
                ))
            })?;
        }

        {
            let _timer = logger.phase("squash");
            rootfs_builder::build_sqfs(&rootfs, &sqfs_tmp, logger)?;
        }

        let metadata = CacheMetadata {
            schema_version: 1,
            ctx_sha: context.ctx_sha.clone(),
            image_ref: cfg.image.clone(),
            image_digest,
            pull_policy_used: pull_policy.as_str().to_string(),
            setup_script_sha256: context.setup_script_sha256.clone(),
            context_manifest_sha256: context.manifest_sha256.clone(),
            compression: "zstd".to_string(),
            created_unix_ms: unix_millis(),
            giftwrap_version: giftwrap::VERSION.to_string(),
            tool_versions: tools.tool_versions.clone(),
        };

        sqfs_cache::write_atomically(paths, &sqfs_tmp, &metadata)
    })();

    match build_result {
        Ok(()) => {
            let _ = fs::remove_dir_all(&work_dir);
            Ok(())
        }
        Err(err) => {
            if logger.verbose() {
                logger.event(format!(
                    "keeping failed work dir for debugging: {}",
                    work_dir.display()
                ));
            } else {
                let _ = fs::remove_dir_all(&work_dir);
            }
            Err(err)
        }
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn check_userns_support() -> Result<(), GiftwrapError> {
    let userns_path = Path::new("/proc/sys/kernel/unprivileged_userns_clone");
    if !userns_path.exists() {
        return Ok(());
    }

    let value = fs::read_to_string(userns_path).map_err(|err| {
        GiftwrapError::runtime(format!("failed to read {}: {err}", userns_path.display()))
    })?;

    if value.trim() == "0" {
        return Err(GiftwrapError::runtime_hint(
            "host kernel does not allow unprivileged user namespaces",
            "set kernel.unprivileged_userns_clone=1 or run on a compatible host",
        ));
    }

    Ok(())
}
