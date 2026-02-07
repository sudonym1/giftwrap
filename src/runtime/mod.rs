pub mod bwrap;

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
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

pub fn merged_env_from_host(overrides: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env = minimal_env_from_host();
    for (key, value) in overrides {
        env.insert(key.clone(), value.clone());
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
    ensure_overlay_layout(spec)?;
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

pub fn reset_overlay(spec: &RunSpec, logger: &Logger) -> Result<(), GiftwrapError> {
    ensure_overlay_root_safe(&spec.overlay_root)?;

    match fs::symlink_metadata(&spec.overlay_root) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(GiftwrapError::runtime(format!(
                    "overlay root is not a directory: {}",
                    spec.overlay_root.display()
                )));
            }

            if remove_overlay_root(&spec.overlay_root, logger)? {
                logger.event(format!(
                    "reset persistent overlay: {}",
                    spec.overlay_root.display()
                ));
            }
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(GiftwrapError::runtime(format!(
            "failed to inspect overlay directory {}: {err}",
            spec.overlay_root.display()
        ))),
    }
}

pub fn reset_all_overlays(build_root: &Path, logger: &Logger) -> Result<usize, GiftwrapError> {
    let state_root = build_root.join(".giftwrap");
    match fs::symlink_metadata(&state_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(GiftwrapError::runtime(format!(
                    "state root cannot be a symlink: {}",
                    state_root.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(GiftwrapError::runtime(format!(
                    "state root is not a directory: {}",
                    state_root.display()
                )));
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            return Err(GiftwrapError::runtime(format!(
                "failed to inspect state root {}: {err}",
                state_root.display()
            )))
        }
    }

    let entries = fs::read_dir(&state_root).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to read state root {}: {err}",
            state_root.display()
        ))
    })?;

    let mut removed = 0usize;

    for entry in entries {
        let entry = entry.map_err(|err| {
            GiftwrapError::runtime(format!(
                "failed to inspect state root entry {}: {err}",
                state_root.display()
            ))
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !looks_like_ctx_sha(&name) {
            continue;
        }

        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(GiftwrapError::runtime(format!(
                    "failed to inspect overlay directory {}: {err}",
                    path.display()
                )))
            }
        };

        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }

        ensure_overlay_root_safe(&path)?;
        if remove_overlay_root(&path, logger)? {
            logger.event(format!("reset persistent overlay: {}", path.display()));
            removed += 1;
        }
    }

    Ok(removed)
}

fn remove_overlay_force(root: &Path) -> Result<(), GiftwrapError> {
    let mut stack = vec![(root.to_path_buf(), false)];

    while let Some((path, visited)) = stack.pop() {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(GiftwrapError::runtime(format!(
                    "failed to inspect overlay path {}: {err}",
                    path.display()
                )))
            }
        };

        if metadata.is_dir() {
            if !visited {
                ensure_dir_access(&path)?;
                let children = read_dir_children(&path)?;
                stack.push((path, true));
                for child in children {
                    stack.push((child, false));
                }
                continue;
            }

            remove_dir_force(&path)?;
            continue;
        }

        remove_file_force(&path)?;
    }

    if root.exists() {
        return Err(GiftwrapError::runtime(format!(
            "overlay directory still exists after force-remove: {}",
            root.display()
        )));
    }

    Ok(())
}

fn remove_overlay_root(path: &Path, logger: &Logger) -> Result<bool, GiftwrapError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            logger.event(format!(
                "remove_dir_all permission denied for {}; falling back to native force-remove",
                path.display()
            ));
            remove_overlay_force(path)?;
            Ok(true)
        }
        Err(err) => Err(GiftwrapError::runtime(format!(
            "failed to remove overlay directory {}: {err}",
            path.display()
        ))),
    }
}

fn read_dir_children(path: &Path) -> Result<Vec<PathBuf>, GiftwrapError> {
    let entries = fs::read_dir(path).or_else(|err| {
        if err.kind() != ErrorKind::PermissionDenied {
            return Err(err);
        }
        ensure_dir_access(path).map_err(|_| err)?;
        fs::read_dir(path)
    });

    match entries {
        Ok(entries) => entries
            .map(|entry| {
                entry.map(|entry| entry.path()).map_err(|err| {
                    GiftwrapError::runtime(format!(
                        "failed to inspect overlay entry in {}: {err}",
                        path.display()
                    ))
                })
            })
            .collect(),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(GiftwrapError::runtime(format!(
            "failed to read overlay directory {}: {err}",
            path.display()
        ))),
    }
}

fn remove_file_force(path: &Path) -> Result<(), GiftwrapError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            ensure_parent_access(path)?;
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
                Err(err) => Err(GiftwrapError::runtime(format!(
                    "failed to remove overlay file {}: {err}",
                    path.display()
                ))),
            }
        }
        Err(err) => Err(GiftwrapError::runtime(format!(
            "failed to remove overlay file {}: {err}",
            path.display()
        ))),
    }
}

fn remove_dir_force(path: &Path) -> Result<(), GiftwrapError> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            ensure_parent_access(path)?;
            ensure_dir_access(path)?;
            match fs::remove_dir(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
                Err(err) => Err(GiftwrapError::runtime(format!(
                    "failed to remove overlay directory {}: {err}",
                    path.display()
                ))),
            }
        }
        Err(err) => Err(GiftwrapError::runtime(format!(
            "failed to remove overlay directory {}: {err}",
            path.display()
        ))),
    }
}

fn ensure_parent_access(path: &Path) -> Result<(), GiftwrapError> {
    if let Some(parent) = path.parent() {
        ensure_dir_access(parent)?;
    }
    Ok(())
}

fn ensure_dir_access(path: &Path) -> Result<(), GiftwrapError> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to inspect overlay directory {}: {err}",
            path.display()
        ))
    })?;

    if !metadata.is_dir() {
        return Ok(());
    }

    let mut perms = metadata.permissions();
    let mode = perms.mode();
    let desired = mode | 0o700;
    if desired != mode {
        perms.set_mode(desired);
        fs::set_permissions(path, perms).map_err(|err| {
            GiftwrapError::runtime(format!(
                "failed to adjust overlay directory permissions {}: {err}",
                path.display()
            ))
        })?;
    }

    Ok(())
}

fn ensure_overlay_layout(spec: &RunSpec) -> Result<(), GiftwrapError> {
    ensure_overlay_root_safe(&spec.overlay_root)?;

    for dir in [&spec.overlay_upper, &spec.overlay_work] {
        fs::create_dir_all(dir).map_err(|err| {
            GiftwrapError::runtime(format!(
                "failed to create overlay directory {}: {err}",
                dir.display()
            ))
        })?;
    }

    Ok(())
}

fn ensure_overlay_root_safe(overlay_root: &Path) -> Result<(), GiftwrapError> {
    match fs::symlink_metadata(overlay_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(GiftwrapError::runtime(format!(
                    "overlay root cannot be a symlink: {}",
                    overlay_root.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(GiftwrapError::runtime(format!(
                    "overlay root is not a directory: {}",
                    overlay_root.display()
                )));
            }
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(GiftwrapError::runtime(format!(
            "failed to inspect overlay root {}: {err}",
            overlay_root.display()
        ))),
    }
}

fn looks_like_ctx_sha(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|ch| ch.is_ascii_hexdigit())
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
