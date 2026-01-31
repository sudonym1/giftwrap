mod internal;

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const SPEC_ENV: &str = "GW_INTERNAL_SPEC";

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(flag) = args.first() {
        if let Some(path) = flag.strip_prefix("--dump-env=") {
            dump_env(Path::new(path))?;
            return Ok(());
        }
        if flag == "--dump-env" {
            let path = args
                .get(1)
                .ok_or_else(|| "Error: --dump-env requires a path".to_string())?;
            if args.len() > 2 {
                return Err("Error: --dump-env accepts a single path".to_string());
            }
            dump_env(Path::new(path))?;
            return Ok(());
        }

        if env::var(SPEC_ENV).is_err() {
            return Err(format!("Error: unknown argument: {flag}"));
        }
    }

    let spec = load_spec()?;
    if spec.protocol_version != internal::INTERNAL_SPEC_VERSION {
        return Err(format!(
            "Error: internal spec version mismatch (expected {}, got {})",
            internal::INTERNAL_SPEC_VERSION,
            spec.protocol_version
        ));
    }
    run_spec(spec)
}

fn load_spec() -> Result<internal::InternalSpec, String> {
    let raw = env::var(SPEC_ENV)
        .map_err(|_| format!("Error: missing {SPEC_ENV} environment variable"))?;
    serde_json::from_str(&raw)
        .map_err(|err| format!("Error: failed to parse internal spec: {err}"))
}

fn run_spec(spec: internal::InternalSpec) -> Result<(), String> {
    env::set_current_dir(&spec.workdir).map_err(|err| {
        format!(
            "Error: failed to enter workdir {}: {err}",
            spec.workdir.display()
        )
    })?;

    setup_user(&spec.user)?;

    let mut env_map = build_base_env(&spec)?;
    env_map.extend(spec.env_overrides.clone());
    env_map.insert(
        "HOME".to_string(),
        spec.user.home.to_string_lossy().into_owned(),
    );
    env_map.remove(SPEC_ENV);

    drop_privileges(spec.user.uid, spec.user.gid)?;

    if let Some(terminfo) = &spec.terminfo {
        install_terminfo(terminfo, &env_map)?;
    }

    let agent_path = current_agent_path();
    let script = build_shell_script(&spec, &agent_path);
    let shell = select_shell(&spec);
    exec_shell(&shell, &script, &env_map)
}

fn build_base_env(spec: &internal::InternalSpec) -> Result<BTreeMap<String, String>, String> {
    if let Some(persist) = &spec.persist_env {
        if persist.restore && persist.path.exists() {
            match load_env(&persist.path) {
                Ok(env_map) => return Ok(env_map),
                Err(err) => {
                    eprintln!(
                        "Warning: failed to restore environment from {}: {err}",
                        persist.path.display()
                    );
                }
            }
        }
    }
    Ok(env::vars().collect())
}

fn setup_user(user: &internal::UserSpec) -> Result<(), String> {
    let base_home = user
        .home
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));

    fs::create_dir_all(&base_home).map_err(|err| {
        format!(
            "Error: failed to create home base {}: {err}",
            base_home.display()
        )
    })?;

    let mut perms = fs::metadata(&base_home)
        .map_err(|err| format!("Error: failed to read {}: {err}", base_home.display()))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&base_home, perms).map_err(|err| {
        format!(
            "Error: failed to set permissions on {}: {err}",
            base_home.display()
        )
    })?;

    run_command_ignore("userdel", &["os76"]);
    run_command_ignore(
        "groupadd",
        &["-g", &user.gid.to_string(), &user.name],
    );
    run_command_ignore(
        "useradd",
        &[
            "-d",
            &user.home.to_string_lossy(),
            "-m",
            "-g",
            &user.gid.to_string(),
            "-u",
            &user.uid.to_string(),
            &user.name,
        ],
    );
    run_command_ignore(
        "sed",
        &[
            "-ir",
            &format!("s/.*{}.*//g", user.name),
            "/etc/sudoers",
        ],
    );

    let mut sudoers = OpenOptions::new()
        .append(true)
        .open("/etc/sudoers")
        .map_err(|err| format!("Error: failed to open /etc/sudoers: {err}"))?;
    writeln!(sudoers, "{} ALL=(ALL) NOPASSWD: ALL", user.name)
        .map_err(|err| format!("Error: failed to update /etc/sudoers: {err}"))?;

    Ok(())
}

fn run_command_ignore(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd).args(args).status();
}

fn drop_privileges(uid: u32, gid: u32) -> Result<(), String> {
    unsafe {
        if libc::setgid(gid as libc::gid_t) != 0 {
            return Err(format!(
                "Error: failed to setgid({gid}): {}",
                std::io::Error::last_os_error()
            ));
        }
        if libc::setuid(uid as libc::uid_t) != 0 {
            return Err(format!(
                "Error: failed to setuid({uid}): {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

fn install_terminfo(
    terminfo: &internal::TerminfoSpec,
    env_map: &BTreeMap<String, String>,
) -> Result<(), String> {
    let home = env_map
        .get("HOME")
        .ok_or_else(|| "Error: HOME is missing from environment".to_string())?;
    let terminfo_dir = Path::new(home).join(".terminfo");
    fs::create_dir_all(&terminfo_dir).map_err(|err| {
        format!(
            "Error: failed to create {}: {err}",
            terminfo_dir.display()
        )
    })?;

    let terminfo_file = Path::new(home).join("terminfo");
    fs::write(&terminfo_file, &terminfo.data).map_err(|err| {
        format!(
            "Error: failed to write {}: {err}",
            terminfo_file.display()
        )
    })?;

    let _ = Command::new("tic")
        .arg(&terminfo_file)
        .envs(env_map)
        .status();

    Ok(())
}

fn current_agent_path() -> String {
    env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "giftwrap-agent".to_string())
}

fn build_shell_script(spec: &internal::InternalSpec, agent_path: &str) -> String {
    let mut cmds = Vec::new();

    if let Some(extra_shell) = &spec.extra_shell {
        cmds.push(format!(
            "source {}",
            shell_escape(&extra_shell.to_string_lossy())
        ));
    }

    if !spec.prefix_cmd.is_empty() {
        cmds.push(format!(
            "{} < /dev/null",
            mk_bash_exe_env(&spec.prefix_cmd)
        ));
    } else if !spec.prefix_cmd_quiet.is_empty() {
        cmds.push(format!(
            "{} < /dev/null > /dev/null 2>&1",
            mk_bash_exe_env(&spec.prefix_cmd_quiet)
        ));
    }

    if !spec.command.is_empty() {
        cmds.push(mk_bash_exe_env(&spec.command));
        cmds.push("drrc=$?".to_string());
    }

    if let Some(persist) = &spec.persist_env {
        if persist.save {
            cmds.push(format!(
                "{} --dump-env {}",
                shell_escape(agent_path),
                shell_escape(&persist.path.to_string_lossy())
            ));
        }
    }

    if !spec.command.is_empty() {
        cmds.push("exit $drrc".to_string());
    }

    cmds.join("; ")
}

fn mk_bash_exe_env(cmds: &[String]) -> String {
    format!("{{ {}; }}", cmds.join(" "))
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

fn select_shell(spec: &internal::InternalSpec) -> String {
    if let Some(shell) = &spec.shell {
        return shell.clone();
    }
    if Path::new("/bin/bash").exists() {
        "/bin/bash".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

fn exec_shell(
    shell: &str,
    script: &str,
    env_map: &BTreeMap<String, String>,
) -> Result<(), String> {
    let err = Command::new(shell)
        .arg("-c")
        .arg(script)
        .env_clear()
        .envs(env_map)
        .exec();
    Err(format!("Error: failed to exec shell: {err}"))
}

fn dump_env(path: &Path) -> Result<(), String> {
    let mut env_map: BTreeMap<String, String> = env::vars().collect();
    env_map.remove("SHLVL");
    let data = serde_json::to_vec(&env_map)
        .map_err(|err| format!("Error: failed to serialize environment: {err}"))?;
    fs::write(path, data)
        .map_err(|err| format!("Error: failed to write {}: {err}", path.display()))
}

fn load_env(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let data = fs::read(path)
        .map_err(|err| format!("Error: failed to read {}: {err}", path.display()))?;
    serde_json::from_slice(&data)
        .map_err(|err| format!("Error: failed to parse {}: {err}", path.display()))
}
