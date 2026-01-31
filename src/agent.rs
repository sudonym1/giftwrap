use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::internal;

const SPEC_ENV: &str = "GW_INTERNAL_SPEC";

pub fn run(args: &[String]) -> Result<(), String> {
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
    serde_json::from_str(&raw).map_err(|err| format!("Error: failed to parse internal spec: {err}"))
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
    if let Some(persist) = &spec.persist_env
        && persist.restore
        && persist.path.exists()
    {
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
    run_command_ignore("groupadd", &["-g", &user.gid.to_string(), &user.name]);
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

    ensure_group_entry(user)?;
    ensure_passwd_entry(user)?;
    ensure_home_dir(user)?;

    let sudoers_path = Path::new("/etc/sudoers");
    if sudoers_path.exists() {
        let sudo_name = lookup_username(user.uid).unwrap_or_else(|| user.name.clone());
        run_command_ignore(
            "sed",
            &["-ir", &format!("s/.*{}.*//g", user.name), "/etc/sudoers"],
        );
        if sudo_name != user.name {
            run_command_ignore(
                "sed",
                &["-ir", &format!("s/.*{}.*//g", sudo_name), "/etc/sudoers"],
            );
        }
        let sudo_target = if sudo_name.is_empty() {
            user.name.as_str()
        } else {
            sudo_name.as_str()
        };
        let mut sudoers = OpenOptions::new()
            .append(true)
            .open(sudoers_path)
            .map_err(|err| format!("Error: failed to open /etc/sudoers: {err}"))?;
        writeln!(sudoers, "{} ALL=(ALL) NOPASSWD: ALL", sudo_target)
            .map_err(|err| format!("Error: failed to update /etc/sudoers: {err}"))?;
    } else {
        eprintln!("Warning: /etc/sudoers not found; skipping sudoers update");
    }

    Ok(())
}

fn run_command_ignore(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd).args(args).status();
}

fn group_state(contents: &str, gid: u32) -> (bool, bool) {
    let mut has_gid = false;
    let mut has_root = false;
    for line in contents.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(':');
        let name = parts.next().unwrap_or("");
        let _ = parts.next();
        let line_gid = parts.next().and_then(|val| val.parse::<u32>().ok());
        if name == "root" || line_gid == Some(0) {
            has_root = true;
        }
        if line_gid == Some(gid) {
            has_gid = true;
        }
        if has_gid && has_root {
            break;
        }
    }
    (has_gid, has_root)
}

fn passwd_state(contents: &str, uid: u32) -> (bool, bool) {
    let mut has_uid = false;
    let mut has_root = false;
    for line in contents.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(':');
        let name = parts.next().unwrap_or("");
        let _ = parts.next();
        let line_uid = parts.next().and_then(|val| val.parse::<u32>().ok());
        if name == "root" || line_uid == Some(0) {
            has_root = true;
        }
        if line_uid == Some(uid) {
            has_uid = true;
        }
        if has_uid && has_root {
            break;
        }
    }
    (has_uid, has_root)
}

fn ensure_group_entry(user: &internal::UserSpec) -> Result<(), String> {
    let group_path = Path::new("/etc/group");
    let mut contents = String::new();
    if group_path.exists() {
        contents = fs::read_to_string(group_path)
            .map_err(|err| format!("Error: failed to read /etc/group: {err}"))?;
    }

    let (has_gid, has_root) = group_state(&contents, user.gid);

    if has_gid {
        return Ok(());
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(group_path)
        .map_err(|err| format!("Error: failed to open /etc/group: {err}"))?;
    if !has_root {
        if !contents.is_empty() && !contents.ends_with('\n') {
            writeln!(file).map_err(|err| format!("Error: failed to write /etc/group: {err}"))?;
        }
        writeln!(file, "root:x:0:")
            .map_err(|err| format!("Error: failed to write /etc/group: {err}"))?;
    }
    writeln!(file, "{}:x:{}:", user.name, user.gid)
        .map_err(|err| format!("Error: failed to write /etc/group: {err}"))?;
    Ok(())
}

fn ensure_passwd_entry(user: &internal::UserSpec) -> Result<(), String> {
    let passwd_path = Path::new("/etc/passwd");
    let mut contents = String::new();
    if passwd_path.exists() {
        contents = fs::read_to_string(passwd_path)
            .map_err(|err| format!("Error: failed to read /etc/passwd: {err}"))?;
    }

    let (has_uid, has_root) = passwd_state(&contents, user.uid);

    if has_uid {
        return Ok(());
    }

    let shell = if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "/bin/sh"
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(passwd_path)
        .map_err(|err| format!("Error: failed to open /etc/passwd: {err}"))?;
    if !has_root {
        if !contents.is_empty() && !contents.ends_with('\n') {
            writeln!(file)
                .map_err(|err| format!("Error: failed to write /etc/passwd: {err}"))?;
        }
        writeln!(file, "root:x:0:0:root:/root:{shell}")
            .map_err(|err| format!("Error: failed to write /etc/passwd: {err}"))?;
    }
    writeln!(
        file,
        "{}:x:{}:{}:{}:{}:{}",
        user.name,
        user.uid,
        user.gid,
        user.name,
        user.home.display(),
        shell
    )
    .map_err(|err| format!("Error: failed to write /etc/passwd: {err}"))?;
    Ok(())
}

fn lookup_username(uid: u32) -> Option<String> {
    unsafe {
        let pwd = libc::getpwuid(uid as libc::uid_t);
        if pwd.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr((*pwd).pw_name)
            .to_string_lossy()
            .into_owned();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }
}

fn ensure_home_dir(user: &internal::UserSpec) -> Result<(), String> {
    fs::create_dir_all(&user.home).map_err(|err| {
        format!(
            "Error: failed to create home {}: {err}",
            user.home.display()
        )
    })?;
    let mut perms = fs::metadata(&user.home)
        .map_err(|err| format!("Error: failed to read {}: {err}", user.home.display()))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&user.home, perms).map_err(|err| {
        format!(
            "Error: failed to set permissions on {}: {err}",
            user.home.display()
        )
    })?;
    chown_path(&user.home, user.uid, user.gid)?;
    Ok(())
}

fn chown_path(path: &Path, uid: u32, gid: u32) -> Result<(), String> {
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("Error: invalid path contains NUL byte: {}", path.display()))?;
    unsafe {
        if libc::chown(c_path.as_ptr(), uid as libc::uid_t, gid as libc::gid_t) != 0 {
            return Err(format!(
                "Error: failed to chown {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
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
    fs::create_dir_all(&terminfo_dir)
        .map_err(|err| format!("Error: failed to create {}: {err}", terminfo_dir.display()))?;

    let terminfo_file = Path::new(home).join("terminfo");
    fs::write(&terminfo_file, &terminfo.data)
        .map_err(|err| format!("Error: failed to write {}: {err}", terminfo_file.display()))?;

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
        .unwrap_or_else(|| "giftwrap".to_string())
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
        cmds.push(format!("{} < /dev/null", mk_bash_exe_env(&spec.prefix_cmd)));
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

    if let Some(persist) = &spec.persist_env && persist.save {
        cmds.push(format!(
            "{} agent --dump-env {}",
            shell_escape(agent_path),
            shell_escape(&persist.path.to_string_lossy())
        ));
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

fn exec_shell(shell: &str, script: &str, env_map: &BTreeMap<String, String>) -> Result<(), String> {
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
    fs::write(path, data).map_err(|err| format!("Error: failed to write {}: {err}", path.display()))
}

fn load_env(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let data =
        fs::read(path).map_err(|err| format!("Error: failed to read {}: {err}", path.display()))?;
    serde_json::from_slice(&data)
        .map_err(|err| format!("Error: failed to parse {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{group_state, passwd_state};

    #[test]
    fn group_state_detects_root_and_gid() {
        let contents = "# comment\nroot:x:0:\nwheel:x:10:root\nuser:x:1000:\n";
        let (has_gid, has_root) = group_state(contents, 1000);
        assert!(has_gid);
        assert!(has_root);
    }

    #[test]
    fn passwd_state_detects_root_and_uid() {
        let contents = "root:x:0:0:root:/root:/bin/sh\nuser:x:1000:1000:User:/home/user:/bin/bash\n";
        let (has_uid, has_root) = passwd_state(contents, 1000);
        assert!(has_uid);
        assert!(has_root);
    }
}
