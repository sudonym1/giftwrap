pub mod bwrap;

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::fs_std::FileExt;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::errors::GiftwrapError;
use crate::log::Logger;
use crate::process::{self, CommandFailureKind};
use crate::runtime::bwrap::{build_argv, RunSpec};

const OVERLAY_MOUNTER: &str = "fuse-overlayfs";

#[derive(Debug, Clone)]
pub struct SharedMountSpec {
    pub ctx_sha: String,
    pub cache_root: PathBuf,
    pub sqfs_path: PathBuf,
    pub state_root: PathBuf,
    pub runtime_root: PathBuf,
    pub runtime_lock: PathBuf,
    pub runtime_leases: PathBuf,
    pub runtime_active: PathBuf,
    pub overlay_root: PathBuf,
    pub overlay_upper: PathBuf,
    pub overlay_work: PathBuf,
    pub lower_mountpoint: PathBuf,
    pub merged_mountpoint: PathBuf,
}

#[derive(Debug)]
struct RuntimeLease {
    lease_path: PathBuf,
    mount: SharedMountSpec,
}

#[derive(Debug, Serialize, Deserialize)]
struct ActiveRuntimeState {
    ctx_sha: String,
    cache_root: PathBuf,
    sqfs_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct LeaseRecord {
    pid: u32,
    start_time_ticks: u64,
    created_unix_ms: u64,
}

pub fn build_shared_mount_spec(
    build_root: &Path,
    cache_root: &Path,
    ctx_sha: &str,
    sqfs_path: &Path,
) -> SharedMountSpec {
    let state_root = build_root.join(".giftwrap");
    let runtime_root = state_root.join("runtime");
    let overlay_root = state_root.join("overlay");
    let mnt_root = cache_root.join("mnt");

    SharedMountSpec {
        ctx_sha: ctx_sha.to_string(),
        cache_root: cache_root.to_path_buf(),
        sqfs_path: sqfs_path.to_path_buf(),
        state_root: state_root.clone(),
        runtime_root: runtime_root.clone(),
        runtime_lock: runtime_root.join("runtime.lock"),
        runtime_leases: runtime_root.join("leases"),
        runtime_active: runtime_root.join("active.json"),
        overlay_root: overlay_root.clone(),
        overlay_upper: overlay_root.join("upper"),
        overlay_work: overlay_root.join("work"),
        lower_mountpoint: mnt_root.join("lower"),
        merged_mountpoint: mnt_root.join("root"),
    }
}

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
    mount: &SharedMountSpec,
    spec: &RunSpec,
    unmount_tool: &str,
    logger: &Logger,
) -> Result<i32, GiftwrapError> {
    let lease = acquire_runtime_lease(mount, unmount_tool, logger)?;

    let run_result = run_bwrap_child(spec, logger);
    let release_result = release_runtime_lease(&lease, unmount_tool, logger);

    match (run_result, release_result) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(run_err), Ok(())) => Err(run_err),
        (Ok(_), Err(release_err)) => Err(release_err),
        (Err(run_err), Err(release_err)) => {
            logger.event(format!("secondary runtime release error: {release_err}"));
            Err(run_err)
        }
    }
}

pub fn reset_overlay(
    mount: &SharedMountSpec,
    unmount_tool: &str,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    ensure_runtime_layout(mount)?;
    let _lock = lock_runtime_state(mount)?;

    let active_leases = prune_stale_leases(mount)?;
    if active_leases > 0 {
        return Err(GiftwrapError::runtime_hint(
            "cannot reset persistent overlay while giftwrap commands are still running",
            "wait for active giftwrap commands to finish, then retry --reset",
        ));
    }

    unmount_shared_roots(mount, unmount_tool, logger)?;
    let _ = fs::remove_file(&mount.runtime_active);

    ensure_overlay_root_safe(&mount.overlay_root)?;
    if remove_overlay_root(&mount.overlay_root, logger)? {
        logger.event(format!(
            "reset persistent overlay: {}",
            mount.overlay_root.display()
        ));
    }

    Ok(())
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

        let should_remove = name == "overlay" || looks_like_ctx_sha(&name);
        if !should_remove {
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

        if metadata.file_type().is_symlink() {
            continue;
        }

        if !metadata.is_dir() {
            return Err(GiftwrapError::runtime(format!(
                "overlay root is not a directory: {}",
                path.display()
            )));
        }

        ensure_overlay_root_safe(&path)?;
        if remove_overlay_root(&path, logger)? {
            logger.event(format!("reset persistent overlay: {}", path.display()));
            removed += 1;
        }
    }

    Ok(removed)
}

fn acquire_runtime_lease(
    mount: &SharedMountSpec,
    unmount_tool: &str,
    logger: &Logger,
) -> Result<RuntimeLease, GiftwrapError> {
    ensure_runtime_layout(mount)?;
    let _lock = lock_runtime_state(mount)?;

    let active_leases = prune_stale_leases(mount)?;

    if active_leases == 0 {
        unmount_shared_roots(mount, unmount_tool, logger)?;
        ensure_overlay_layout(mount)?;

        mount_sqfs(&mount.sqfs_path, &mount.lower_mountpoint, logger)?;
        if let Err(err) = mount_overlay_root(mount, logger) {
            let _ = unmount_sqfs(&mount.lower_mountpoint, unmount_tool, logger);
            return Err(err);
        }

        write_active_state(
            &mount.runtime_active,
            &ActiveRuntimeState {
                ctx_sha: mount.ctx_sha.clone(),
                cache_root: mount.cache_root.clone(),
                sqfs_path: mount.sqfs_path.clone(),
            },
        )?;
    } else {
        let active = read_active_state(&mount.runtime_active)?;
        if active.ctx_sha != mount.ctx_sha {
            return Err(GiftwrapError::runtime_hint(
                format!(
                    "cannot run context {} while context {} is active in this workspace",
                    short_ctx(&mount.ctx_sha),
                    short_ctx(&active.ctx_sha),
                ),
                "wait for active giftwrap commands to finish before switching context",
            ));
        }

        if active.cache_root != mount.cache_root {
            return Err(GiftwrapError::runtime_hint(
                format!(
                    "active runtime uses cache root {}, but this run requested {}",
                    active.cache_root.display(),
                    mount.cache_root.display(),
                ),
                "retry with the same --cache-dir as the active giftwrap command",
            ));
        }

        let mounted = load_mount_points();
        if !mounted.contains(&mount.lower_mountpoint) || !mounted.contains(&mount.merged_mountpoint)
        {
            return Err(GiftwrapError::runtime_hint(
                "runtime lease state is inconsistent: shared mounts are missing",
                "wait for active commands to finish, then retry",
            ));
        }
    }

    let lease_path = create_lease_file(&mount.runtime_leases)?;

    Ok(RuntimeLease {
        lease_path,
        mount: mount.clone(),
    })
}

fn release_runtime_lease(
    lease: &RuntimeLease,
    unmount_tool: &str,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    ensure_runtime_layout(&lease.mount)?;
    let _lock = lock_runtime_state(&lease.mount)?;

    let _ = fs::remove_file(&lease.lease_path);

    let active_leases = prune_stale_leases(&lease.mount)?;
    if active_leases == 0 {
        unmount_shared_roots(&lease.mount, unmount_tool, logger)?;
        let _ = fs::remove_file(&lease.mount.runtime_active);
    }

    Ok(())
}

fn ensure_runtime_layout(mount: &SharedMountSpec) -> Result<(), GiftwrapError> {
    if let Ok(metadata) = fs::symlink_metadata(&mount.state_root) {
        if metadata.file_type().is_symlink() {
            return Err(GiftwrapError::runtime(format!(
                "state root cannot be a symlink: {}",
                mount.state_root.display()
            )));
        }
    }

    fs::create_dir_all(&mount.state_root).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to create state directory {}: {err}",
            mount.state_root.display()
        ))
    })?;

    for dir in [
        &mount.runtime_root,
        &mount.runtime_leases,
        &mount.cache_root,
        &mount.cache_root.join("mnt"),
    ] {
        fs::create_dir_all(dir).map_err(|err| {
            GiftwrapError::runtime(format!(
                "failed to create runtime directory {}: {err}",
                dir.display()
            ))
        })?;
    }

    Ok(())
}

fn lock_runtime_state(mount: &SharedMountSpec) -> Result<File, GiftwrapError> {
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&mount.runtime_lock)
        .map_err(|err| {
            GiftwrapError::runtime(format!(
                "failed to open runtime lock {}: {err}",
                mount.runtime_lock.display()
            ))
        })?;

    lock_file
        .lock_exclusive()
        .map_err(|err| GiftwrapError::runtime(format!("failed to lock runtime state: {err}")))?;

    Ok(lock_file)
}

fn prune_stale_leases(runtime: &SharedMountSpec) -> Result<usize, GiftwrapError> {
    let entries = fs::read_dir(&runtime.runtime_leases).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to read runtime lease directory {}: {err}",
            runtime.runtime_leases.display()
        ))
    })?;

    let mut active = 0usize;

    for entry in entries {
        let entry = entry.map_err(|err| {
            GiftwrapError::runtime(format!("failed to inspect runtime lease entry: {err}"))
        })?;
        let path = entry.path();

        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(GiftwrapError::runtime(format!(
                    "failed to inspect runtime lease {}: {err}",
                    path.display()
                )))
            }
        };

        if metadata.file_type().is_symlink() || !metadata.is_file() {
            let _ = fs::remove_file(&path);
            continue;
        }

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(_) => {
                let _ = fs::remove_file(&path);
                continue;
            }
        };

        let lease = match serde_json::from_slice::<LeaseRecord>(&bytes) {
            Ok(lease) => lease,
            Err(_) => {
                let _ = fs::remove_file(&path);
                continue;
            }
        };

        if is_process_alive(lease.pid, lease.start_time_ticks) {
            active += 1;
        } else {
            let _ = fs::remove_file(&path);
        }
    }

    Ok(active)
}

fn create_lease_file(lease_dir: &Path) -> Result<PathBuf, GiftwrapError> {
    let pid = std::process::id();
    let start_time_ticks = read_process_start_time_ticks(pid).ok_or_else(|| {
        GiftwrapError::runtime(format!(
            "failed to inspect /proc/{pid}/stat for runtime lease"
        ))
    })?;

    for attempt in 0..10 {
        let lease_path = lease_dir.join(format!("{pid}-{}-{attempt}.lease", unix_millis()));
        let lease = LeaseRecord {
            pid,
            start_time_ticks,
            created_unix_ms: unix_millis(),
        };
        let payload = serde_json::to_vec(&lease)
            .map_err(|err| GiftwrapError::runtime(format!("failed to encode lease: {err}")))?;

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lease_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(&payload).map_err(|err| {
                    GiftwrapError::runtime(format!(
                        "failed to write runtime lease {}: {err}",
                        lease_path.display()
                    ))
                })?;
                return Ok(lease_path);
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(GiftwrapError::runtime(format!(
                    "failed to create runtime lease {}: {err}",
                    lease_path.display()
                )))
            }
        }
    }

    Err(GiftwrapError::runtime(
        "failed to allocate unique runtime lease filename",
    ))
}

fn is_process_alive(pid: u32, expected_start_time_ticks: u64) -> bool {
    read_process_start_time_ticks(pid).is_some_and(|start| start == expected_start_time_ticks)
}

fn read_process_start_time_ticks(pid: u32) -> Option<u64> {
    let contents = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close_paren = contents.rfind(')')?;
    let rest = contents.get(close_paren + 2..)?;
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let start_time = fields.get(19)?;
    start_time.parse::<u64>().ok()
}

fn write_active_state(path: &Path, state: &ActiveRuntimeState) -> Result<(), GiftwrapError> {
    let parent = path.parent().ok_or_else(|| {
        GiftwrapError::runtime(format!(
            "runtime state file has no parent directory: {}",
            path.display()
        ))
    })?;

    fs::create_dir_all(parent).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to create runtime state directory {}: {err}",
            parent.display()
        ))
    })?;

    let tmp = parent.join(format!("active.tmp.{}", std::process::id()));
    let payload = serde_json::to_vec_pretty(state)
        .map_err(|err| GiftwrapError::runtime(format!("failed to encode runtime state: {err}")))?;

    fs::write(&tmp, payload).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to write runtime state temp file {}: {err}",
            tmp.display()
        ))
    })?;

    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(GiftwrapError::runtime(format!(
            "failed to update runtime state {}: {err}",
            path.display()
        )));
    }

    Ok(())
}

fn read_active_state(path: &Path) -> Result<ActiveRuntimeState, GiftwrapError> {
    let bytes = fs::read(path).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to read runtime state {}: {err}",
            path.display()
        ))
    })?;

    serde_json::from_slice::<ActiveRuntimeState>(&bytes).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to parse runtime state {}: {err}",
            path.display()
        ))
    })
}

fn unmount_shared_roots(
    mount: &SharedMountSpec,
    unmount_tool: &str,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    let mounted = load_mount_points();
    if mounted.contains(&mount.merged_mountpoint) {
        unmount_sqfs(&mount.merged_mountpoint, unmount_tool, logger)?;
    }

    let mounted = load_mount_points();
    if mounted.contains(&mount.lower_mountpoint) {
        unmount_sqfs(&mount.lower_mountpoint, unmount_tool, logger)?;
    }

    Ok(())
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

fn ensure_overlay_layout(spec: &SharedMountSpec) -> Result<(), GiftwrapError> {
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

fn mount_overlay_root(spec: &SharedMountSpec, logger: &Logger) -> Result<(), GiftwrapError> {
    fs::create_dir_all(&spec.merged_mountpoint).map_err(|err| {
        GiftwrapError::runtime(format!(
            "failed to create mountpoint {}: {err}",
            spec.merged_mountpoint.display()
        ))
    })?;

    let options = format!(
        "lowerdir={},upperdir={},workdir={}",
        spec.lower_mountpoint.display(),
        spec.overlay_upper.display(),
        spec.overlay_work.display(),
    );

    let args = vec![
        "-o".to_string(),
        options,
        spec.merged_mountpoint.display().to_string(),
    ];

    process::run_checked(
        OVERLAY_MOUNTER,
        &args,
        CommandFailureKind::Runtime,
        "mount shared overlay",
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

fn load_mount_points() -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    if let Ok(contents) = fs::read_to_string("/proc/self/mounts") {
        for line in contents.lines() {
            let mut parts = line.split_whitespace();
            let _src = parts.next();
            if let Some(mountpoint) = parts.next() {
                set.insert(PathBuf::from(mountpoint));
            }
        }
    }
    set
}

fn short_ctx(ctx_sha: &str) -> String {
    ctx_sha.chars().take(12).collect()
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
