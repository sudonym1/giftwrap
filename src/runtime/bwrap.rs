use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::PathBuf;

use crate::errors::GiftwrapError;

#[derive(Debug, Clone)]
pub struct RunSpec {
    pub host_uid: u32,
    pub host_gid: u32,
    pub build_root: PathBuf,
    pub workdir: PathBuf,
    pub mountpoint: PathBuf,
    pub env: BTreeMap<String, String>,
    pub argv: Vec<String>,
}

pub fn build_argv(spec: &RunSpec) -> Vec<String> {
    let mut args = vec![
        "--die-with-parent".to_string(),
        "--new-session".to_string(),
        "--unshare-ipc".to_string(),
        "--unshare-pid".to_string(),
        "--unshare-uts".to_string(),
        "--unshare-cgroup-try".to_string(),
        "--share-net".to_string(),
        "--unshare-user-try".to_string(),
        "--uid".to_string(),
        spec.host_uid.to_string(),
        "--gid".to_string(),
        spec.host_gid.to_string(),
        "--overlay-src".to_string(),
        spec.mountpoint.display().to_string(),
        "--tmp-overlay".to_string(),
        "/".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--bind".to_string(),
        spec.build_root.display().to_string(),
        spec.build_root.display().to_string(),
        "--chdir".to_string(),
        spec.workdir.display().to_string(),
        "--clearenv".to_string(),
    ];

    for (key, value) in &spec.env {
        args.push("--setenv".to_string());
        args.push(key.clone());
        args.push(value.clone());
    }

    args.push("--".to_string());
    args.extend(spec.argv.iter().cloned());
    args
}

pub fn exec(spec: &RunSpec) -> Result<Infallible, GiftwrapError> {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let args = build_argv(spec);
    let err = Command::new("bwrap").args(&args).exec();
    Err(GiftwrapError::runtime(format!(
        "failed to exec bwrap: {err}"
    )))
}
