mod cli;
mod config;
mod context;
mod exec;
mod internal;
mod podman_cli;

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    use std::collections::BTreeMap;
    use std::env;
    use std::io::IsTerminal;

    let orig_cwd =
        env::current_dir().map_err(|err| format!("Error: failed to resolve cwd: {err}"))?;
    let args: Vec<String> = env::args().skip(1).collect();
    let (cli_opts, user_cmd) = cli::parse_args(&args).map_err(|err| err.to_string())?;

    let config = config::load_from(&orig_cwd).map_err(|err| err.to_string())?;
    let root_dir = config.root_dir.clone();
    env::set_current_dir(&root_dir)
        .map_err(|err| format!("Error: failed to enter build root: {err}"))?;

    let mut params = config.params.clone();
    params
        .entry("extra_args".to_string())
        .or_insert_with(Vec::new);

    let context = context::load_from_config(&root_dir, &params).map_err(|err| err.to_string())?;
    let mut ctx_sha = context.as_ref().map(|ctx| ctx.sha.clone());
    if let Some(forced) = &cli_opts.use_ctx {
        if context.is_none() {
            return Err("Error: context sha us unused by this configuration".to_string());
        }
        ctx_sha = Some(forced.clone());
    }

    if matches!(cli_opts.action, cli::CliAction::PrintContext) {
        if let Some(sha) = ctx_sha {
            println!("{sha}");
            return Ok(());
        }
        return Err("Error: context sha us unused by this configuration".to_string());
    }

    let mut image = params
        .get("gw_container")
        .and_then(|vals| vals.first())
        .ok_or_else(|| "Error: gw_container must be specified".to_string())?
        .to_string();
    if let Some(sha) = &ctx_sha {
        image = format!("{image}:{sha}");
    }
    if let Some(override_image) = cli_opts.override_image.clone() {
        image = override_image;
    }

    if matches!(cli_opts.action, cli::CliAction::PrintImage) {
        println!("{image}");
        return Ok(());
    }

    if matches!(cli_opts.action, cli::CliAction::ShowConfig) {
        println!("{:#?}", params);
        return Ok(());
    }

    if matches!(cli_opts.action, cli::CliAction::Help) {
        print_help();
        return Ok(());
    }

    if let Some(hook) = params.get("prelaunch_hook") {
        run_hook(hook, &root_dir)?;
    }

    if cli_opts.rebuild {
        println!("Rebuilding container {image}");
        exec::build_image(&image, &root_dir).map_err(|err| err.to_string())?;
    }

    let mut env_overrides = BTreeMap::new();
    env_overrides.insert(
        "GW_BUILD_ROOT".to_string(),
        root_dir.to_string_lossy().into_owned(),
    );

    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let interactive = true;
    let tty = stdin_tty && stdout_tty;
    if tty {
        if let Ok(term) = env::var("TERM") {
            env_overrides.insert("TERM".to_string(), term);
        }
    }

    if let Some(env_keys) = params.get("env_overrides") {
        for key in env_keys {
            if let Ok(val) = env::var(key) {
                env_overrides.insert(key.to_string(), val);
            }
        }
    }

    let mut mounts = Vec::new();
    let mut mount_target = root_dir.clone();
    let mut cd_to = orig_cwd.clone();
    if let Some(mount_to) = params.get("mount_to").and_then(|vals| vals.first()) {
        mount_target = PathBuf::from(mount_to);
        cd_to = PathBuf::from(mount_to);
    }
    if let Some(cd_override) = params.get("cd_to").and_then(|vals| vals.first()) {
        cd_to = PathBuf::from(cd_override);
    }
    mounts.push(internal::Mount {
        source: root_dir.clone(),
        target: mount_target,
        read_only: false,
        options: Vec::new(),
    });

    if let Some(extra_shares) = params.get("extra_shares") {
        for share in extra_shares {
            let Some(share) = expand_share(share) else {
                continue;
            };
            if let Some(mount) = parse_share(&share, &root_dir) {
                mounts.push(mount);
            }
        }
    }

    if params.contains_key("share_git_dir") {
        if let Some(git_mount) = share_git_dir(&root_dir) {
            mounts.push(git_mount);
        }
    }

    let mut extra_args = cli_opts.extra_args.clone();
    let mut config_extra_args = params.get("extra_args").cloned().unwrap_or_default();
    if !cli_opts.runtime_args.is_empty() {
        config_extra_args.extend(cli_opts.runtime_args.clone());
    }
    extra_args.extend(config_extra_args);

    let hostname = mkhostname(&image);
    let container_spec = internal::ContainerSpec {
        image,
        hostname: Some(hostname),
        mounts,
        env: env_overrides,
        workdir: Some(cd_to),
        user: Some("root".to_string()),
        extra_hosts: params.get("extra_hosts").cloned().unwrap_or_default(),
        privileged: true,
        init: true,
        remove: true,
        interactive,
        tty,
        entrypoint: None,
        command: user_cmd.argv.clone(),
        extra_args,
    };

    if matches!(cli_opts.action, cli::CliAction::PrintCommand) {
        let mut cmd = vec!["podman".to_string()];
        let args = podman_cli::build_run_args(&container_spec).map_err(|err| err.to_string())?;
        cmd.extend(args);
        for arg in cmd {
            println!("++++ {arg}");
        }
        return Ok(());
    }

    exec::run_container(&container_spec).map_err(|err| err.to_string())
}

fn print_help() {
    println!(
        r#"
GW Flags:
    print: print the runtime command instead of executing it
    ctx: print the context sha
    print-image: print the image
    use-ctx: force a particular context sha
    img: force a particular image
    rebuild: rebuild the container image
    show-config: dump the parameters
    extra-args: add extra args to the runtime invocation
"#
    );
}

fn run_hook(hook: &[String], root_dir: &Path) -> Result<(), String> {
    if hook.is_empty() {
        return Ok(());
    }
    let status = Command::new(&hook[0])
        .args(&hook[1..])
        .current_dir(root_dir)
        .status()
        .map_err(|err| format!("Error: failed to run prelaunch_hook: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Error: prelaunch_hook failed (exit {})",
            format_exit_status(&status)
        ))
    }
}

fn mkhostname(image: &str) -> String {
    let base = image.rsplit('/').next().unwrap_or(image);
    let mut out = String::new();
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.len() > 63 {
        out.truncate(63);
    }
    out
}

fn expand_share(value: &str) -> Option<String> {
    if let Some(rest) = value.strip_prefix('$') {
        return std::env::var(rest).ok();
    }
    Some(value.to_string())
}

fn parse_share(share: &str, root_dir: &Path) -> Option<internal::Mount> {
    let parts: Vec<&str> = share.split(':').collect();
    if parts.is_empty() {
        return None;
    }
    if parts.len() == 1 {
        let source = abs_path(parts[0], root_dir);
        return Some(internal::Mount {
            source: source.clone(),
            target: source,
            read_only: false,
            options: Vec::new(),
        });
    }
    let source = abs_path(parts[0], root_dir);
    let target = PathBuf::from(parts[1]);
    let options = if parts.len() >= 3 {
        parts[2]
            .split(',')
            .filter(|opt| !opt.is_empty())
            .map(|opt| opt.to_string())
            .collect()
    } else {
        Vec::new()
    };
    Some(internal::Mount {
        source,
        target,
        read_only: false,
        options,
    })
}

fn share_git_dir(root_dir: &Path) -> Option<internal::Mount> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--git-common-dir")
        .current_dir(root_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let git_dir = abs_path(raw.trim(), root_dir);
    if git_dir.starts_with(root_dir) {
        return None;
    }
    Some(internal::Mount {
        source: git_dir.clone(),
        target: git_dir,
        read_only: false,
        options: Vec::new(),
    })
}

fn abs_path(path: &str, root_dir: &Path) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        root_dir.join(candidate)
    }
}

fn format_exit_status(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => code.to_string(),
        None => "signal".to_string(),
    }
}
