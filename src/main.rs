mod agent;
mod cli;
mod config;
mod context;
mod exec;
mod internal;
mod podman_cli;

use std::ffi::CStr;
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

    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "agent") {
        return agent::run(&args[1..]);
    }

    let orig_cwd =
        env::current_dir().map_err(|err| format!("Error: failed to resolve cwd: {err}"))?;
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

    let image = select_image(
        &params,
        ctx_sha.as_deref(),
        cli_opts.override_image.as_deref(),
    )?;

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

    if let Some(rebuild_image) = rebuild_plan(cli_opts.rebuild, &image) {
        println!("Rebuilding container {rebuild_image}");
        exec::build_image(&rebuild_image, &root_dir).map_err(|err| err.to_string())?;
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
    let mut terminfo = None;
    if tty && let Ok(term) = env::var("TERM") {
        env_overrides.insert("TERM".to_string(), term.clone());
        terminfo = Some(load_terminfo(&term)?);
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
        target: mount_target.clone(),
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

    if params.contains_key("share_git_dir") && let Some(git_mount) = share_git_dir(&root_dir) {
        mounts.push(git_mount);
    }

    let mut extra_shell_path = None;
    if let Some(extra_shell) = params.get("extra_shell").and_then(|vals| vals.first()) {
        let resolved = resolve_path(extra_shell, &root_dir);
        mounts.push(internal::Mount {
            source: resolved.clone(),
            target: resolved.clone(),
            read_only: false,
            options: Vec::new(),
        });
        extra_shell_path = Some(resolved);
    }

    let agent_override = params
        .get("gw_agent")
        .and_then(|vals| vals.first())
        .map(|val| val.as_str());
    let (agent_source, agent_target) =
        resolve_giftwrap_mount(agent_override, &root_dir, &mount_target)?;
    mounts.push(internal::Mount {
        source: agent_source,
        target: agent_target.clone(),
        read_only: true,
        options: Vec::new(),
    });

    let mut extra_args = cli_opts.extra_args.clone();
    let mut config_extra_args = params.get("extra_args").cloned().unwrap_or_default();
    if !cli_opts.runtime_args.is_empty() {
        config_extra_args.extend(cli_opts.runtime_args.clone());
    }
    extra_args.extend(config_extra_args);

    let uid = unsafe { libc::getuid() } as u32;
    let gid = unsafe { libc::getgid() } as u32;
    let internal_spec = build_internal_spec(
        &root_dir,
        cd_to,
        user_cmd.argv.clone(),
        env_overrides,
        &params,
        terminfo,
        extra_shell_path,
        uid,
        gid,
    );

    let internal_spec_json = serde_json::to_string(&internal_spec)
        .map_err(|err| format!("Error: failed to serialize internal spec: {err}"))?;

    let agent_path = agent_target.to_string_lossy().into_owned();

    let mut container_env = BTreeMap::new();
    container_env.insert("GW_INTERNAL_SPEC".to_string(), internal_spec_json);

    let hostname = mkhostname(&image);
    let container_spec = internal::ContainerSpec {
        image,
        hostname: Some(hostname),
        mounts,
        env: container_env,
        workdir: None,
        user: Some("root".to_string()),
        extra_hosts: params.get("extra_hosts").cloned().unwrap_or_default(),
        privileged: true,
        init: true,
        remove: true,
        interactive,
        tty,
        entrypoint: Some(vec![agent_path]),
        command: vec!["agent".to_string()],
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

#[allow(clippy::too_many_arguments)]
fn build_internal_spec(
    root_dir: &Path,
    workdir: PathBuf,
    command: Vec<String>,
    env_overrides: std::collections::BTreeMap<String, String>,
    params: &std::collections::HashMap<String, Vec<String>>,
    terminfo: Option<internal::TerminfoSpec>,
    extra_shell: Option<PathBuf>,
    uid: u32,
    gid: u32,
) -> internal::InternalSpec {
    let user_name = resolve_username(uid);
    let user_home = build_home(&user_name);
    let persist_env = params
        .get("persist_environment")
        .and_then(|vals| vals.first())
        .map(|path| internal::PersistEnvSpec {
            path: resolve_real_path(path, root_dir),
            restore: true,
            save: true,
        });

    internal::InternalSpec {
        protocol_version: internal::INTERNAL_SPEC_VERSION,
        workdir,
        root_dir: root_dir.to_path_buf(),
        user: internal::UserSpec {
            name: user_name,
            uid,
            gid,
            home: user_home,
        },
        env_overrides,
        persist_env,
        terminfo,
        command,
        shell: None,
        extra_shell,
        prefix_cmd: params.get("prefix_cmd").cloned().unwrap_or_default(),
        prefix_cmd_quiet: params.get("prefix_cmd_quiet").cloned().unwrap_or_default(),
    }
}

fn select_image(
    params: &std::collections::HashMap<String, Vec<String>>,
    ctx_sha: Option<&str>,
    override_image: Option<&str>,
) -> Result<String, String> {
    let mut image = params
        .get("gw_container")
        .and_then(|vals| vals.first())
        .ok_or_else(|| "Error: gw_container must be specified".to_string())?
        .to_string();
    if let Some(sha) = ctx_sha {
        image = format!("{image}:{sha}");
    }
    if let Some(override_image) = override_image {
        image = override_image.to_string();
    }
    Ok(image)
}

fn rebuild_plan(rebuild: bool, image: &str) -> Option<String> {
    if rebuild {
        Some(image.to_string())
    } else {
        None
    }
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

fn resolve_path(path: &str, root_dir: &Path) -> PathBuf {
    abs_path(path, root_dir)
}

fn resolve_real_path(path: &str, root_dir: &Path) -> PathBuf {
    let candidate = abs_path(path, root_dir);
    std::fs::canonicalize(&candidate).unwrap_or(candidate)
}

fn resolve_giftwrap_mount(
    agent_override: Option<&str>,
    root_dir: &Path,
    mount_target: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    let (target, hint) = match agent_override {
        Some(value) => {
            let value_path = PathBuf::from(value);
            if value_path.is_absolute() {
                (value_path.clone(), Some(value_path))
            } else {
                (
                    mount_target.join(value),
                    Some(resolve_real_path(value, root_dir)),
                )
            }
        }
        None => {
            let default_path = PathBuf::from("/usr/local/bin/giftwrap");
            (default_path.clone(), Some(default_path))
        }
    };

    let hint_source = hint.as_ref().and_then(|path| {
        if path.is_file() {
            Some(path.clone())
        } else {
            None
        }
    });
    let host_source = if agent_override.is_none() {
        find_musl_binary(root_dir, "giftwrap")
            .or_else(|| hint_source.clone())
            .or_else(|| find_adjacent_binary("giftwrap"))
            .or_else(|| find_in_path("giftwrap"))
    } else {
        hint_source
            .or_else(|| find_musl_binary(root_dir, "giftwrap"))
            .or_else(|| find_adjacent_binary("giftwrap"))
            .or_else(|| find_in_path("giftwrap"))
    };

    match host_source {
        Some(source) => Ok((source, target)),
        None => {
            let hint_display = hint
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<none>".to_string());
            Err(format!(
                "Error: failed to locate giftwrap on host (checked {hint_display}, musl target, giftwrap-adjacent, and PATH). Build giftwrap (musl) or set gw_agent to a valid host path."
            ))
        }
    }
}

fn find_musl_binary(root_dir: &Path, binary: &str) -> Option<PathBuf> {
    let target_root = root_dir.join("target");
    let preferred = target_root.join(format!("{}-unknown-linux-musl", std::env::consts::ARCH));
    if let Some(found) = find_musl_binary_in_target(&preferred, binary) {
        return Some(found);
    }

    let entries = std::fs::read_dir(&target_root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path == preferred {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let name = name.to_string_lossy();
        if !name.ends_with("linux-musl") {
            continue;
        }
        if let Some(found) = find_musl_binary_in_target(&path, binary) {
            return Some(found);
        }
    }
    None
}

fn find_musl_binary_in_target(target_dir: &Path, binary: &str) -> Option<PathBuf> {
    let debug = target_dir.join("debug").join(binary);
    if debug.is_file() {
        return Some(debug);
    }
    let release = target_dir.join("release").join(binary);
    if release.is_file() {
        return Some(release);
    }
    None
}

fn find_adjacent_binary(binary: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join(binary);
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn resolve_username(uid: u32) -> String {
    if let Ok(name) = std::env::var("USER") && !name.is_empty() {
        return name;
    }
    if let Ok(name) = std::env::var("LOGNAME") && !name.is_empty() {
        return name;
    }
    unsafe {
        let pwd = libc::getpwuid(uid as libc::uid_t);
        if !pwd.is_null() {
            let name = CStr::from_ptr((*pwd).pw_name)
                .to_string_lossy()
                .into_owned();
            if !name.is_empty() {
                return name;
            }
        }
    }
    uid.to_string()
}

fn build_home(user: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/dr-tmp-home-{user}/{user}"))
}

fn load_terminfo(term: &str) -> Result<internal::TerminfoSpec, String> {
    let output = Command::new("infocmp")
        .arg(term)
        .output()
        .map_err(|err| format!("Error: failed to run infocmp: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "Error: infocmp failed (exit {})",
            format_exit_status(&output.status)
        ));
    }
    Ok(internal::TerminfoSpec {
        term: term.to_string(),
        data: output.stdout,
    })
}

fn format_exit_status(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => code.to_string(),
        None => "signal".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        abs_path, build_internal_spec, expand_share, mkhostname, parse_share, rebuild_plan,
        resolve_path, resolve_real_path, select_image, share_git_dir,
    };
    use crate::internal;
    use serde_json::Value;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().expect("env lock poisoned")
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    fn params_with_image(image: &str) -> HashMap<String, Vec<String>> {
        let mut params = HashMap::new();
        params.insert("gw_container".to_string(), vec![image.to_string()]);
        params
    }

    #[test]
    fn mkhostname_sanitizes_and_truncates() {
        let hostname = mkhostname("registry.local/org/my_app:latest");
        assert_eq!(hostname, "my-app-latest");

        let long = format!("repo/{}", "a".repeat(80));
        let hostname = mkhostname(&long);
        assert_eq!(hostname, "a".repeat(63));
    }

    #[test]
    fn expand_share_resolves_env_and_literals() {
        let _lock = lock_env();
        let prior = std::env::var("GW_TEST_SHARE").ok();

        unsafe {
            std::env::set_var("GW_TEST_SHARE", "/tmp/share");
        }
        assert_eq!(
            expand_share("$GW_TEST_SHARE").as_deref(),
            Some("/tmp/share")
        );

        unsafe {
            std::env::remove_var("GW_TEST_SHARE");
        }
        assert!(expand_share("$GW_TEST_SHARE").is_none());
        assert_eq!(expand_share("literal").as_deref(), Some("literal"));

        if let Some(value) = prior {
            unsafe {
                std::env::set_var("GW_TEST_SHARE", value);
            }
        } else {
            unsafe {
                std::env::remove_var("GW_TEST_SHARE");
            }
        }
    }

    #[test]
    fn select_image_defaults_to_container_value() {
        let params = params_with_image("registry.local/app");
        let image = select_image(&params, None, None).expect("select image");
        assert_eq!(image, "registry.local/app");
    }

    #[test]
    fn select_image_appends_context_sha() {
        let params = params_with_image("registry.local/app");
        let image = select_image(&params, Some("deadbeef"), None).expect("select image");
        assert_eq!(image, "registry.local/app:deadbeef");
    }

    #[test]
    fn select_image_override_wins_over_context() {
        let params = params_with_image("registry.local/app");
        let image = select_image(&params, Some("deadbeef"), Some("override/app:tag"))
            .expect("select image");
        assert_eq!(image, "override/app:tag");
    }

    #[test]
    fn select_image_errors_without_gw_container() {
        let params = HashMap::new();
        let err = select_image(&params, None, None).expect_err("missing gw_container");
        assert_eq!(err, "Error: gw_container must be specified");
    }

    #[test]
    fn rebuild_plan_returns_image_when_enabled() {
        let image = "registry/app:tag";
        assert_eq!(rebuild_plan(false, image), None);
        assert_eq!(rebuild_plan(true, image), Some(image.to_string()));
    }

    #[test]
    fn parse_share_defaults_target_to_source() {
        let root = TempDir::new().expect("tempdir");
        let mount = parse_share("src", root.path()).expect("parse_share failed");
        let expected = root.path().join("src");
        assert_eq!(mount.source, expected);
        assert_eq!(mount.target, expected);
        assert!(!mount.read_only);
        assert!(mount.options.is_empty());
    }

    #[test]
    fn parse_share_parses_target_and_options() {
        let root = TempDir::new().expect("tempdir");
        let mount = parse_share("src:/dest:ro,z", root.path()).expect("parse_share failed");
        assert_eq!(mount.source, root.path().join("src"));
        assert_eq!(mount.target, PathBuf::from("/dest"));
        assert_eq!(mount.options, vec!["ro".to_string(), "z".to_string()]);
    }

    #[test]
    fn abs_and_resolve_path_keep_absolute_or_join_root() {
        let root = TempDir::new().expect("tempdir");
        assert_eq!(abs_path("rel", root.path()), root.path().join("rel"));
        assert_eq!(resolve_path("rel", root.path()), root.path().join("rel"));
        assert_eq!(abs_path("/abs", root.path()), PathBuf::from("/abs"));
    }

    #[test]
    fn resolve_real_path_returns_candidate_on_missing() {
        let root = TempDir::new().expect("tempdir");
        assert_eq!(
            resolve_real_path("missing", root.path()),
            root.path().join("missing")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_real_path_canonicalizes_symlinks() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let root = TempDir::new().expect("tempdir");
        let real = root.path().join("real");
        fs::create_dir(&real).expect("create real dir");
        let link = root.path().join("link");
        symlink(&real, &link).expect("symlink");

        let resolved = resolve_real_path("link", root.path());
        assert_eq!(resolved, real.canonicalize().expect("canonicalize"));
    }

    #[test]
    fn internal_spec_serializes_expected_shape() {
        let _lock = lock_env();
        let prior_user = std::env::var("USER").ok();
        let prior_logname = std::env::var("LOGNAME").ok();

        unsafe {
            std::env::set_var("USER", "gw-test");
            std::env::remove_var("LOGNAME");
        }

        let root = TempDir::new().expect("tempdir");
        let root_dir = root.path().canonicalize().expect("canonicalize root");
        let workdir = root_dir.join("work");
        let extra_shell = root_dir.join("extra.sh");

        let mut env_overrides = BTreeMap::new();
        env_overrides.insert(
            "GW_BUILD_ROOT".to_string(),
            root_dir.to_string_lossy().into_owned(),
        );
        env_overrides.insert("GW_EXTRA".to_string(), "1".to_string());

        let mut params = HashMap::new();
        params.insert(
            "persist_environment".to_string(),
            vec!["persist.env".to_string()],
        );
        params.insert(
            "prefix_cmd".to_string(),
            vec!["/usr/bin/env".to_string(), "FOO=bar".to_string()],
        );

        let terminfo = internal::TerminfoSpec {
            term: "xterm-256color".to_string(),
            data: vec![1, 2, 3],
        };

        let spec = build_internal_spec(
            &root_dir,
            workdir.clone(),
            vec!["echo".to_string(), "hi".to_string()],
            env_overrides,
            &params,
            Some(terminfo),
            Some(extra_shell.clone()),
            123,
            456,
        );

        let value = serde_json::to_value(&spec).expect("serialize internal spec");
        let obj = value.as_object().expect("internal spec object");

        let expected: HashSet<&str> = [
            "protocol_version",
            "workdir",
            "root_dir",
            "user",
            "env_overrides",
            "persist_env",
            "terminfo",
            "command",
            "shell",
            "extra_shell",
            "prefix_cmd",
            "prefix_cmd_quiet",
        ]
        .into_iter()
        .collect();
        let keys: HashSet<&str> = obj.keys().map(|key| key.as_str()).collect();
        assert_eq!(keys, expected);

        assert_eq!(
            obj.get("protocol_version").and_then(Value::as_u64),
            Some(internal::INTERNAL_SPEC_VERSION as u64)
        );
        assert_eq!(
            obj.get("workdir").and_then(Value::as_str),
            Some(workdir.to_string_lossy().as_ref())
        );
        assert_eq!(
            obj.get("root_dir").and_then(Value::as_str),
            Some(root_dir.to_string_lossy().as_ref())
        );
        assert_eq!(obj.get("shell"), Some(&Value::Null));
        assert_eq!(
            obj.get("extra_shell").and_then(Value::as_str),
            Some(extra_shell.to_string_lossy().as_ref())
        );

        let command = obj
            .get("command")
            .and_then(Value::as_array)
            .expect("command array");
        assert_eq!(command, &vec![Value::from("echo"), Value::from("hi")]);

        let env = obj
            .get("env_overrides")
            .and_then(Value::as_object)
            .expect("env overrides object");
        assert_eq!(
            env.get("GW_BUILD_ROOT").and_then(Value::as_str),
            Some(root_dir.to_string_lossy().as_ref())
        );
        assert_eq!(env.get("GW_EXTRA").and_then(Value::as_str), Some("1"));

        let persist = obj
            .get("persist_env")
            .and_then(Value::as_object)
            .expect("persist env object");
        assert_eq!(
            persist.get("path").and_then(Value::as_str),
            Some(root_dir.join("persist.env").to_string_lossy().as_ref())
        );
        assert_eq!(persist.get("restore").and_then(Value::as_bool), Some(true));
        assert_eq!(persist.get("save").and_then(Value::as_bool), Some(true));

        let terminfo = obj
            .get("terminfo")
            .and_then(Value::as_object)
            .expect("terminfo object");
        assert_eq!(
            terminfo.get("term").and_then(Value::as_str),
            Some("xterm-256color")
        );
        let data = terminfo
            .get("data")
            .and_then(Value::as_array)
            .expect("terminfo data array");
        assert_eq!(data, &vec![Value::from(1), Value::from(2), Value::from(3)]);

        let prefix_cmd = obj
            .get("prefix_cmd")
            .and_then(Value::as_array)
            .expect("prefix cmd array");
        assert_eq!(
            prefix_cmd,
            &vec![Value::from("/usr/bin/env"), Value::from("FOO=bar")]
        );
        let prefix_quiet = obj
            .get("prefix_cmd_quiet")
            .and_then(Value::as_array)
            .expect("prefix cmd quiet array");
        assert!(prefix_quiet.is_empty());

        if let Some(value) = prior_user {
            unsafe {
                std::env::set_var("USER", value);
            }
        } else {
            unsafe {
                std::env::remove_var("USER");
            }
        }
        if let Some(value) = prior_logname {
            unsafe {
                std::env::set_var("LOGNAME", value);
            }
        } else {
            unsafe {
                std::env::remove_var("LOGNAME");
            }
        }
    }

    #[test]
    fn internal_spec_sets_user_uid_gid_and_home() {
        let _lock = lock_env();
        let prior_user = std::env::var("USER").ok();
        let prior_logname = std::env::var("LOGNAME").ok();

        unsafe {
            std::env::set_var("USER", "gw-user");
            std::env::remove_var("LOGNAME");
        }

        let root = TempDir::new().expect("tempdir");
        let root_dir = root.path().canonicalize().expect("canonicalize root");
        let workdir = root_dir.join("work");

        let spec = build_internal_spec(
            &root_dir,
            workdir,
            vec!["true".to_string()],
            BTreeMap::new(),
            &HashMap::new(),
            None,
            None,
            42,
            1000,
        );

        assert_eq!(spec.user.name, "gw-user");
        assert_eq!(spec.user.uid, 42);
        assert_eq!(spec.user.gid, 1000);
        assert_eq!(
            spec.user.home,
            PathBuf::from("/tmp/dr-tmp-home-gw-user/gw-user")
        );

        if let Some(value) = prior_user {
            unsafe {
                std::env::set_var("USER", value);
            }
        } else {
            unsafe {
                std::env::remove_var("USER");
            }
        }
        if let Some(value) = prior_logname {
            unsafe {
                std::env::set_var("LOGNAME", value);
            }
        } else {
            unsafe {
                std::env::remove_var("LOGNAME");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn internal_spec_persist_env_canonicalizes_symlink() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let root = TempDir::new().expect("tempdir");
        let root_dir = root.path().canonicalize().expect("canonicalize root");
        let real = root_dir.join("real");
        fs::create_dir(&real).expect("create real dir");
        let link = root_dir.join("link");
        symlink(&real, &link).expect("symlink");

        let mut params = HashMap::new();
        params.insert("persist_environment".to_string(), vec!["link".to_string()]);

        let spec = build_internal_spec(
            &root_dir,
            root_dir.join("work"),
            vec!["true".to_string()],
            BTreeMap::new(),
            &params,
            None,
            None,
            0,
            0,
        );

        let persist = spec.persist_env.expect("persist env");
        assert_eq!(
            persist.path,
            real.canonicalize().expect("canonicalize real")
        );
    }

    #[test]
    fn share_git_dir_skips_repo_inside_root() {
        if !git_available() {
            return;
        }

        let root = TempDir::new().expect("tempdir");
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.path())
            .status()
            .expect("git init failed");
        assert!(status.success());

        let mount = share_git_dir(root.path());
        assert!(mount.is_none());
    }

    #[test]
    fn share_git_dir_mounts_external_gitdir() {
        if !git_available() {
            return;
        }

        let root = TempDir::new().expect("tempdir");
        let git_dir = TempDir::new().expect("tempdir");
        let status = Command::new("git")
            .args(["init", "-q", "--separate-git-dir"])
            .arg(git_dir.path())
            .current_dir(root.path())
            .status()
            .expect("git init failed");
        assert!(status.success());

        let mount = share_git_dir(root.path()).expect("expected external git dir mount");
        assert_eq!(mount.source, git_dir.path());
        assert_eq!(mount.target, git_dir.path());
        assert!(!mount.read_only);
        assert!(mount.options.is_empty());
    }
}
