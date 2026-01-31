use std::fmt;
use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::internal::{ContainerSpec, Mount};

#[derive(Debug)]
pub struct PodmanError {
    message: String,
}

impl PodmanError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PodmanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PodmanError {}

pub fn build_image(image: &str, context_dir: &Path) -> Result<(), PodmanError> {
    let status = Command::new("podman")
        .arg("build")
        .arg("-t")
        .arg(image)
        .arg(context_dir)
        .status()
        .map_err(|err| PodmanError::new(format!("Error: failed to launch runtime build: {err}")))?;

    if status.success() {
        Ok(())
    } else {
        Err(PodmanError::new(format!(
            "Error: runtime build failed (exit {})",
            format_exit_status(&status)
        )))
    }
}

pub fn inspect_image(image: &str) -> Result<bool, PodmanError> {
    let status = Command::new("podman")
        .arg("image")
        .arg("exists")
        .arg(image)
        .status()
        .map_err(|err| {
            PodmanError::new(format!(
                "Error: failed to launch runtime image exists: {err}"
            ))
        })?;

    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(PodmanError::new(format!(
            "Error: runtime image exists failed (exit {})",
            format_exit_status(&status)
        ))),
    }
}

pub fn build_run_args(spec: &ContainerSpec) -> Result<Vec<String>, PodmanError> {
    let mut args = Vec::new();
    args.push("run".to_string());

    if spec.interactive {
        args.push("-i".to_string());
    }
    if spec.tty {
        args.push("-t".to_string());
    }

    if spec.remove {
        args.push("--rm".to_string());
    }

    if spec.init {
        args.push("--init".to_string());
    }
    if spec.privileged {
        args.push("--privileged=true".to_string());
    }

    if let Some(hostname) = &spec.hostname {
        args.push("-h".to_string());
        args.push(hostname.clone());
    }

    for host in &spec.extra_hosts {
        args.push("--add-host".to_string());
        args.push(host.clone());
    }

    for mount in &spec.mounts {
        args.push("-v".to_string());
        args.push(mount_to_arg(mount));
    }

    for (key, value) in &spec.env {
        args.push("--env".to_string());
        args.push(format!("{key}={value}"));
    }

    if let Some(workdir) = &spec.workdir {
        args.push("-w".to_string());
        args.push(workdir.to_string_lossy().into_owned());
    }

    if let Some(user) = &spec.user {
        args.push("-u".to_string());
        args.push(user.clone());
    }

    if let Some(entrypoint) = &spec.entrypoint {
        match entrypoint.as_slice() {
            [] => {}
            [single] => {
                args.push("--entrypoint".to_string());
                args.push(single.clone());
            }
            _ => {
                return Err(PodmanError::new(
                    "Error: entrypoint must be a single argv element",
                ));
            }
        }
    }

    for extra in &spec.extra_args {
        args.push(extra.clone());
    }

    args.push(spec.image.clone());
    args.extend(spec.command.iter().cloned());

    Ok(args)
}

#[cfg(unix)]
pub fn exec_run(spec: &ContainerSpec) -> Result<(), PodmanError> {
    use std::os::unix::process::CommandExt;

    let args = build_run_args(spec)?;
    let err = Command::new("podman").args(args).exec();
    Err(PodmanError::new(format!(
        "Error: failed to exec runtime run: {err}"
    )))
}

#[cfg(not(unix))]
pub fn exec_run(_spec: &ContainerSpec) -> Result<(), PodmanError> {
    Err(PodmanError::new(
        "Error: runtime exec is only supported on unix platforms",
    ))
}

fn mount_to_arg(mount: &Mount) -> String {
    let mut options: Vec<String> = mount
        .options
        .iter()
        .filter(|opt| !opt.is_empty())
        .cloned()
        .collect();
    if mount.read_only && !options.iter().any(|opt| opt == "ro") {
        options.push("ro".to_string());
    }

    let mut arg = format!(
        "{}:{}",
        mount.source.to_string_lossy(),
        mount.target.to_string_lossy()
    );
    if !options.is_empty() {
        arg.push(':');
        arg.push_str(&options.join(","));
    }
    arg
}

fn format_exit_status(status: &ExitStatus) -> String {
    match status.code() {
        Some(code) => code.to_string(),
        None => "signal".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::build_run_args;
    use crate::internal::{ContainerSpec, Mount};

    fn base_spec() -> ContainerSpec {
        ContainerSpec {
            image: "example:latest".to_string(),
            hostname: None,
            mounts: Vec::new(),
            env: BTreeMap::new(),
            workdir: None,
            user: None,
            extra_hosts: Vec::new(),
            privileged: false,
            init: false,
            remove: false,
            interactive: false,
            tty: false,
            entrypoint: None,
            command: Vec::new(),
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn build_run_args_orders_flags_and_values() {
        let mut spec = base_spec();
        spec.image = "registry/app:tag".to_string();
        spec.interactive = true;
        spec.tty = true;
        spec.remove = true;
        spec.init = true;
        spec.privileged = true;
        spec.hostname = Some("gw-host".to_string());
        spec.extra_hosts = vec![
            "host.docker.internal:host-gateway".to_string(),
            "db:10.0.0.2".to_string(),
        ];
        spec.mounts = vec![
            Mount {
                source: PathBuf::from("/src"),
                target: PathBuf::from("/workspace"),
                read_only: false,
                options: vec!["z".to_string()],
            },
            Mount {
                source: PathBuf::from("/data"),
                target: PathBuf::from("/data"),
                read_only: true,
                options: vec!["Z".to_string()],
            },
        ];
        spec.env.insert("B".to_string(), "2".to_string());
        spec.env.insert("A".to_string(), "1".to_string());
        spec.workdir = Some(PathBuf::from("/work"));
        spec.user = Some("1000:1000".to_string());
        spec.entrypoint = Some(vec!["/bin/sh".to_string()]);
        spec.extra_args = vec![
            "--security-opt=label=disable".to_string(),
            "--pids-limit=100".to_string(),
        ];
        spec.command = vec!["bash".to_string(), "-lc".to_string(), "true".to_string()];

        let args = build_run_args(&spec).expect("build_run_args failed");
        assert_eq!(
            args,
            vec![
                "run",
                "-i",
                "-t",
                "--rm",
                "--init",
                "--privileged=true",
                "-h",
                "gw-host",
                "--add-host",
                "host.docker.internal:host-gateway",
                "--add-host",
                "db:10.0.0.2",
                "-v",
                "/src:/workspace:z",
                "-v",
                "/data:/data:Z,ro",
                "--env",
                "A=1",
                "--env",
                "B=2",
                "-w",
                "/work",
                "-u",
                "1000:1000",
                "--entrypoint",
                "/bin/sh",
                "--security-opt=label=disable",
                "--pids-limit=100",
                "registry/app:tag",
                "bash",
                "-lc",
                "true",
            ]
        );
    }

    #[test]
    fn build_run_args_skips_empty_entrypoint() {
        let mut spec = base_spec();
        spec.entrypoint = Some(Vec::new());
        spec.image = "busybox".to_string();
        spec.command = vec!["echo".to_string(), "ok".to_string()];

        let args = build_run_args(&spec).expect("build_run_args failed");
        assert_eq!(args, vec!["run", "busybox", "echo", "ok"]);
    }

    #[test]
    fn build_run_args_rejects_multi_element_entrypoint() {
        let mut spec = base_spec();
        spec.entrypoint = Some(vec!["/bin/sh".to_string(), "-c".to_string()]);

        let err = build_run_args(&spec)
            .err()
            .expect("expected build_run_args to fail");
        assert_eq!(
            err.to_string(),
            "Error: entrypoint must be a single argv element"
        );
    }

    #[test]
    fn build_run_args_keeps_ro_option_once() {
        let mut spec = base_spec();
        spec.image = "busybox".to_string();
        spec.mounts = vec![Mount {
            source: PathBuf::from("/src"),
            target: PathBuf::from("/dest"),
            read_only: true,
            options: vec!["ro".to_string(), "Z".to_string()],
        }];

        let args = build_run_args(&spec).expect("build_run_args failed");
        assert_eq!(args, vec!["run", "-v", "/src:/dest:ro,Z", "busybox"]);
    }
}
