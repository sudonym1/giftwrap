pub mod bwrap;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::errors::GiftwrapError;
use crate::log::Logger;
use crate::process::{self, CommandFailureKind};
use crate::runtime::bwrap::{build_argv, RunSpec};

pub fn minimal_env_from_host() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();

    for key in ["HOME", "USER", "LOGNAME", "PATH", "TERM"] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }

    env
}

pub fn run_with_mount(
    sqfs_path: &Path,
    mountpoint: &Path,
    spec: &RunSpec,
    unmount_tool: &str,
    logger: &Logger,
) -> Result<i32, GiftwrapError> {
    mount_sqfs(sqfs_path, mountpoint, logger)?;

    let run_result = run_bwrap_child(spec, logger);
    let unmount_result = unmount_sqfs(mountpoint, unmount_tool, logger);

    match (run_result, unmount_result) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(run_err), Ok(())) => Err(run_err),
        (Ok(_), Err(unmount_err)) => Err(unmount_err),
        (Err(run_err), Err(unmount_err)) => {
            logger.event(format!("secondary unmount error: {unmount_err}"));
            Err(run_err)
        }
    }
}

fn mount_sqfs(sqfs_path: &Path, mountpoint: &Path, logger: &Logger) -> Result<(), GiftwrapError> {
    fs::create_dir_all(mountpoint).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to create mountpoint {}: {err}",
            mountpoint.display()
        ))
    })?;

    let args = vec![
        sqfs_path.display().to_string(),
        mountpoint.display().to_string(),
    ];
    process::run_checked(
        "squashfuse",
        &args,
        CommandFailureKind::Runtime,
        "mount squashfs",
        logger,
    )
}

fn unmount_sqfs(
    mountpoint: &Path,
    unmount_tool: &str,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    let args = if unmount_tool == "fusermount3" {
        vec!["-u".to_string(), mountpoint.display().to_string()]
    } else {
        vec![mountpoint.display().to_string()]
    };

    process::run_checked(
        unmount_tool,
        &args,
        CommandFailureKind::Runtime,
        "unmount squashfs",
        logger,
    )
}

fn run_bwrap_child(spec: &RunSpec, logger: &Logger) -> Result<i32, GiftwrapError> {
    let args = build_argv(spec);
    logger.command("bwrap", &args);

    let mut child = Command::new("bwrap")
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0)
        .spawn()
        .map_err(|err| GiftwrapError::runtime(format!("failed to spawn bwrap: {err}")))?;

    let child_pid = child.id() as i32;
    let mut signals = Signals::new([SIGINT, SIGTERM]).map_err(|err| {
        GiftwrapError::runtime(format!("failed to register signal handlers: {err}"))
    })?;
    let handle = signals.handle();

    let forwarder = thread::spawn(move || {
        for signal in signals.forever() {
            let mapped = match signal {
                SIGINT => Some(Signal::SIGINT),
                SIGTERM => Some(Signal::SIGTERM),
                _ => None,
            };

            if let Some(sig) = mapped {
                let _ = killpg(Pid::from_raw(child_pid), sig);
            }
        }
    });

    let status = child
        .wait()
        .map_err(|err| GiftwrapError::runtime(format!("failed waiting on bwrap: {err}")))?;

    handle.close();
    let _ = forwarder.join();

    let exit_code = match status.code() {
        Some(code) => code,
        None => status.signal().map_or(1, |signal| 128 + signal),
    };

    Ok(exit_code)
}
