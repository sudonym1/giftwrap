use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};

use crate::errors::GiftwrapError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullPolicy {
    Missing,
    Always,
    Never,
}

impl PullPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachePaths {
    pub cache_root: PathBuf,
    pub ctx_sha: String,
    pub sqfs: PathBuf,
    pub meta: PathBuf,
    pub lock: PathBuf,
    pub mountpoint: PathBuf,
    pub work_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub schema_version: u32,
    pub ctx_sha: String,
    pub image_ref: String,
    pub image_digest: String,
    pub pull_policy_used: String,
    pub setup_script_sha256: String,
    pub context_manifest_sha256: String,
    pub compression: String,
    pub created_unix_ms: u64,
    pub giftwrap_version: String,
    pub tool_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct GcOptions {
    pub dry_run: bool,
    pub max_age_days: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct GcReport {
    pub removed: usize,
    pub messages: Vec<String>,
}

#[derive(Debug)]
enum GcAction {
    RemoveFile(PathBuf, &'static str),
    RemoveDir(PathBuf, &'static str),
}

pub fn default_cache_root() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".giftwrap/cache");
    }

    PathBuf::from(".giftwrap/cache")
}

pub fn resolve_paths(cache_root: &Path, ctx_sha: &str) -> CachePaths {
    CachePaths {
        cache_root: cache_root.to_path_buf(),
        ctx_sha: ctx_sha.to_string(),
        sqfs: cache_root.join(format!("{ctx_sha}.sqfs")),
        meta: cache_root.join(format!("{ctx_sha}.json")),
        lock: cache_root.join("locks").join(format!("{ctx_sha}.lock")),
        mountpoint: cache_root.join("mnt").join(ctx_sha),
        work_root: cache_root.join("work"),
    }
}

pub fn ensure_layout(paths: &CachePaths) -> Result<(), GiftwrapError> {
    fs::create_dir_all(&paths.cache_root).map_err(|err| {
        GiftwrapError::cache(format!(
            "failed to create cache root {}: {err}",
            paths.cache_root.display()
        ))
    })?;

    for dir in [
        paths.cache_root.join("locks"),
        paths.cache_root.join("work"),
        paths.cache_root.join("mnt"),
    ] {
        fs::create_dir_all(&dir).map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to create cache directory {}: {err}",
                dir.display()
            ))
        })?;
    }

    Ok(())
}

pub fn is_valid(paths: &CachePaths, policy: PullPolicy) -> Result<bool, GiftwrapError> {
    if policy == PullPolicy::Always {
        return Ok(false);
    }

    let sqfs_exists = paths.sqfs.is_file();
    let meta_exists = paths.meta.is_file();

    if !sqfs_exists || !meta_exists {
        if policy == PullPolicy::Never {
            return Err(GiftwrapError::build(format!(
                "cache artifact missing for {} with pull policy 'never'",
                paths.ctx_sha
            )));
        }
        return Ok(false);
    }

    let metadata = read_metadata(&paths.meta)?;
    validate_metadata(paths, &metadata)?;
    Ok(true)
}

pub fn with_lock<T, F>(paths: &CachePaths, timeout: Duration, f: F) -> Result<T, GiftwrapError>
where
    F: FnOnce() -> Result<T, GiftwrapError>,
{
    if let Some(parent) = paths.lock.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to create lock directory {}: {err}",
                parent.display()
            ))
        })?;
    }

    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.lock)
        .map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to open lock file {}: {err}",
                paths.lock.display()
            ))
        })?;

    acquire_lock(&file, timeout, &paths.ctx_sha)?;
    let result = f();
    let _ = file.unlock();
    result
}

pub fn write_atomically(
    paths: &CachePaths,
    sqfs_tmp: &Path,
    meta: &CacheMetadata,
) -> Result<(), GiftwrapError> {
    ensure_layout(paths)?;

    let meta_tmp = paths.meta.with_extension("json.tmp");
    let meta_bytes = serde_json::to_vec_pretty(meta)
        .map_err(|err| GiftwrapError::cache(format!("failed to serialize metadata: {err}")))?;
    fs::write(&meta_tmp, meta_bytes).map_err(|err| {
        GiftwrapError::cache(format!(
            "failed to write metadata temp file {}: {err}",
            meta_tmp.display()
        ))
    })?;

    let sqfs_backup = paths.sqfs.with_extension("sqfs.bak");
    let meta_backup = paths.meta.with_extension("json.bak");

    let had_sqfs = backup_existing(&paths.sqfs, &sqfs_backup)?;
    let had_meta = backup_existing(&paths.meta, &meta_backup)?;

    let commit_result = (|| {
        fs::rename(sqfs_tmp, &paths.sqfs).map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to move squashfs into cache {}: {err}",
                paths.sqfs.display()
            ))
        })?;

        fs::rename(&meta_tmp, &paths.meta).map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to move metadata into cache {}: {err}",
                paths.meta.display()
            ))
        })?;

        Ok(())
    })();

    if let Err(err) = commit_result {
        rollback_if_needed(&paths.sqfs, &sqfs_backup, had_sqfs)?;
        rollback_if_needed(&paths.meta, &meta_backup, had_meta)?;
        let _ = fs::remove_file(&meta_tmp);
        return Err(err);
    }

    let _ = fs::remove_file(&sqfs_backup);
    let _ = fs::remove_file(&meta_backup);
    Ok(())
}

pub fn gc(cache_root: &Path, options: &GcOptions) -> Result<GcReport, GiftwrapError> {
    fs::create_dir_all(cache_root).map_err(|err| {
        GiftwrapError::cache(format!(
            "failed to create cache root {}: {err}",
            cache_root.display()
        ))
    })?;

    let mut actions = Vec::new();
    let now = SystemTime::now();
    let grace_period = Duration::from_secs(5 * 60);
    let stale_work_age = Duration::from_secs(24 * 60 * 60);
    let max_age = options
        .max_age_days
        .map(|days| Duration::from_secs(days * 24 * 60 * 60));

    collect_stale_work(cache_root, now, stale_work_age, grace_period, &mut actions)?;
    collect_stale_mounts(cache_root, now, grace_period, &mut actions)?;
    collect_orphans(cache_root, now, grace_period, &mut actions)?;
    if let Some(max_age) = max_age {
        collect_old_valid(cache_root, now, max_age, grace_period, &mut actions)?;
    }

    let mut removed = 0usize;
    let mut messages = Vec::new();

    for action in actions {
        match action {
            GcAction::RemoveFile(path, reason) => {
                messages.push(format!("remove file {} ({reason})", path.display()));
                if !options.dry_run {
                    fs::remove_file(&path).map_err(|err| {
                        GiftwrapError::cache(format!(
                            "failed to remove file {}: {err}",
                            path.display()
                        ))
                    })?;
                    removed += 1;
                }
            }
            GcAction::RemoveDir(path, reason) => {
                messages.push(format!("remove dir {} ({reason})", path.display()));
                if !options.dry_run {
                    fs::remove_dir_all(&path).map_err(|err| {
                        GiftwrapError::cache(format!(
                            "failed to remove directory {}: {err}",
                            path.display()
                        ))
                    })?;
                    removed += 1;
                }
            }
        }
    }

    Ok(GcReport { removed, messages })
}

fn collect_stale_work(
    cache_root: &Path,
    now: SystemTime,
    max_age: Duration,
    grace_period: Duration,
    actions: &mut Vec<GcAction>,
) -> Result<(), GiftwrapError> {
    let work_root = cache_root.join("work");
    let entries = match fs::read_dir(&work_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(GiftwrapError::cache(format!(
                "failed to read work cache {}: {err}",
                work_root.display()
            )))
        }
    };

    for entry in entries {
        let entry = entry
            .map_err(|err| GiftwrapError::cache(format!("failed to inspect work entry: {err}")))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| {
            GiftwrapError::cache(format!("failed to stat {}: {err}", path.display()))
        })?;

        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }

        if is_recent(metadata.modified().ok(), now, grace_period)
            || !is_older_than(metadata.modified().ok(), now, max_age)
        {
            continue;
        }

        let ctx_sha = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split('-').next())
            .unwrap_or_default()
            .to_string();

        if !ctx_sha.is_empty() && lock_held(cache_root, &ctx_sha)? {
            continue;
        }

        actions.push(GcAction::RemoveDir(path, "stale work dir"));
    }

    Ok(())
}

fn collect_stale_mounts(
    cache_root: &Path,
    now: SystemTime,
    grace_period: Duration,
    actions: &mut Vec<GcAction>,
) -> Result<(), GiftwrapError> {
    let mnt_root = cache_root.join("mnt");
    let entries = match fs::read_dir(&mnt_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(GiftwrapError::cache(format!(
                "failed to read mount cache {}: {err}",
                mnt_root.display()
            )))
        }
    };

    let mount_points = load_mount_points();

    for entry in entries {
        let entry = entry
            .map_err(|err| GiftwrapError::cache(format!("failed to inspect mount entry: {err}")))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| {
            GiftwrapError::cache(format!("failed to stat {}: {err}", path.display()))
        })?;

        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }

        if is_recent(metadata.modified().ok(), now, grace_period) {
            continue;
        }

        if mount_points.contains(&path) {
            continue;
        }

        let ctx_sha = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();

        if !ctx_sha.is_empty() && lock_held(cache_root, &ctx_sha)? {
            continue;
        }

        actions.push(GcAction::RemoveDir(path, "stale mount dir"));
    }

    Ok(())
}

fn collect_orphans(
    cache_root: &Path,
    now: SystemTime,
    grace_period: Duration,
    actions: &mut Vec<GcAction>,
) -> Result<(), GiftwrapError> {
    let entries = fs::read_dir(cache_root).map_err(|err| {
        GiftwrapError::cache(format!(
            "failed to read cache directory {}: {err}",
            cache_root.display()
        ))
    })?;

    let mut metas = Vec::new();
    let mut sqfs = Vec::new();

    for entry in entries {
        let entry = entry
            .map_err(|err| GiftwrapError::cache(format!("failed to inspect cache entry: {err}")))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| {
            GiftwrapError::cache(format!("failed to stat {}: {err}", path.display()))
        })?;

        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }

        if is_recent(metadata.modified().ok(), now, grace_period) {
            continue;
        }

        match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") => metas.push(path),
            Some("sqfs") => sqfs.push(path),
            _ => {}
        }
    }

    for meta_path in metas {
        let Some(ctx_sha) = file_stem_str(&meta_path) else {
            continue;
        };

        if lock_held(cache_root, &ctx_sha)? {
            continue;
        }

        let sqfs_path = cache_root.join(format!("{ctx_sha}.sqfs"));
        if !sqfs_path.exists() {
            actions.push(GcAction::RemoveFile(meta_path, "orphan metadata"));
        }
    }

    for sqfs_path in sqfs {
        let Some(ctx_sha) = file_stem_str(&sqfs_path) else {
            continue;
        };

        if lock_held(cache_root, &ctx_sha)? {
            continue;
        }

        let meta_path = cache_root.join(format!("{ctx_sha}.json"));
        if !meta_path.exists() {
            actions.push(GcAction::RemoveFile(sqfs_path, "orphan sqfs"));
        }
    }

    Ok(())
}

fn collect_old_valid(
    cache_root: &Path,
    now: SystemTime,
    max_age: Duration,
    grace_period: Duration,
    actions: &mut Vec<GcAction>,
) -> Result<(), GiftwrapError> {
    let entries = fs::read_dir(cache_root).map_err(|err| {
        GiftwrapError::cache(format!(
            "failed to read cache directory {}: {err}",
            cache_root.display()
        ))
    })?;

    for entry in entries {
        let entry = entry
            .map_err(|err| GiftwrapError::cache(format!("failed to inspect cache entry: {err}")))?;
        let sqfs_path = entry.path();

        if sqfs_path.extension().and_then(|ext| ext.to_str()) != Some("sqfs") {
            continue;
        }

        let sqfs_metadata = fs::symlink_metadata(&sqfs_path).map_err(|err| {
            GiftwrapError::cache(format!("failed to stat {}: {err}", sqfs_path.display()))
        })?;

        if sqfs_metadata.file_type().is_symlink()
            || !sqfs_metadata.is_file()
            || is_recent(sqfs_metadata.modified().ok(), now, grace_period)
            || !is_older_than(sqfs_metadata.modified().ok(), now, max_age)
        {
            continue;
        }

        let Some(ctx_sha) = file_stem_str(&sqfs_path) else {
            continue;
        };

        if lock_held(cache_root, &ctx_sha)? {
            continue;
        }

        let meta_path = cache_root.join(format!("{ctx_sha}.json"));
        if !meta_path.exists() {
            continue;
        }

        let meta_metadata = fs::symlink_metadata(&meta_path).map_err(|err| {
            GiftwrapError::cache(format!("failed to stat {}: {err}", meta_path.display()))
        })?;

        if meta_metadata.file_type().is_symlink()
            || !meta_metadata.is_file()
            || is_recent(meta_metadata.modified().ok(), now, grace_period)
            || !is_older_than(meta_metadata.modified().ok(), now, max_age)
        {
            continue;
        }

        actions.push(GcAction::RemoveFile(sqfs_path, "expired artifact"));
        actions.push(GcAction::RemoveFile(meta_path, "expired artifact"));
    }

    dedup_actions(actions);
    Ok(())
}

fn dedup_actions(actions: &mut Vec<GcAction>) {
    let mut seen = HashSet::new();
    actions.retain(|action| match action {
        GcAction::RemoveFile(path, _) | GcAction::RemoveDir(path, _) => seen.insert(path.clone()),
    });
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

fn is_recent(modified: Option<SystemTime>, now: SystemTime, period: Duration) -> bool {
    modified
        .and_then(|mtime| now.duration_since(mtime).ok())
        .is_some_and(|age| age < period)
}

fn is_older_than(modified: Option<SystemTime>, now: SystemTime, period: Duration) -> bool {
    modified
        .and_then(|mtime| now.duration_since(mtime).ok())
        .is_some_and(|age| age > period)
}

fn lock_held(cache_root: &Path, ctx_sha: &str) -> Result<bool, GiftwrapError> {
    let lock_path = cache_root.join("locks").join(format!("{ctx_sha}.lock"));
    let file = match OpenOptions::new().read(true).write(true).open(&lock_path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(GiftwrapError::cache(format!(
                "failed to inspect lock file {}: {err}",
                lock_path.display()
            )))
        }
    };

    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(false)
        }
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(true),
        Err(err) => Err(GiftwrapError::cache(format!(
            "failed to check lock state for {}: {err}",
            lock_path.display()
        ))),
    }
}

fn acquire_lock(file: &File, timeout: Duration, ctx_sha: &str) -> Result<(), GiftwrapError> {
    let start = Instant::now();

    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                if start.elapsed() >= timeout {
                    return Err(GiftwrapError::cache_lock_timeout(ctx_sha.to_string()));
                }
                thread::sleep(Duration::from_millis(200));
            }
            Err(err) => {
                return Err(GiftwrapError::cache(format!(
                    "failed to acquire cache lock: {err}"
                )));
            }
        }
    }
}

fn read_metadata(path: &Path) -> Result<CacheMetadata, GiftwrapError> {
    let bytes = fs::read(path).map_err(|err| {
        GiftwrapError::cache(format!(
            "failed to read cache metadata {}: {err}",
            path.display()
        ))
    })?;

    serde_json::from_slice(&bytes).map_err(|err| {
        GiftwrapError::cache(format!(
            "failed to parse cache metadata {}: {err}",
            path.display()
        ))
    })
}

fn validate_metadata(paths: &CachePaths, metadata: &CacheMetadata) -> Result<(), GiftwrapError> {
    if metadata.schema_version != 1 {
        return Err(GiftwrapError::cache(format!(
            "unsupported metadata schema_version {}",
            metadata.schema_version
        )));
    }

    if metadata.ctx_sha != paths.ctx_sha {
        return Err(GiftwrapError::cache(format!(
            "cache metadata ctx_sha mismatch: expected {}, got {}",
            paths.ctx_sha, metadata.ctx_sha
        )));
    }

    if metadata.compression != "zstd" {
        return Err(GiftwrapError::cache(format!(
            "unsupported cache compression {}",
            metadata.compression
        )));
    }

    Ok(())
}

fn backup_existing(path: &Path, backup_path: &Path) -> Result<bool, GiftwrapError> {
    if !path.exists() {
        return Ok(false);
    }

    if backup_path.exists() {
        fs::remove_file(backup_path).map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to remove stale backup {}: {err}",
                backup_path.display()
            ))
        })?;
    }

    fs::rename(path, backup_path).map_err(|err| {
        GiftwrapError::cache(format!("failed to stage backup {}: {err}", path.display()))
    })?;

    Ok(true)
}

fn rollback_if_needed(
    path: &Path,
    backup_path: &Path,
    had_backup: bool,
) -> Result<(), GiftwrapError> {
    if !had_backup {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        return Ok(());
    }

    if path.exists() {
        fs::remove_file(path).map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to remove partial artifact {}: {err}",
                path.display()
            ))
        })?;
    }

    if backup_path.exists() {
        fs::rename(backup_path, path).map_err(|err| {
            GiftwrapError::cache(format!(
                "failed to restore artifact {}: {err}",
                path.display()
            ))
        })?;
    }

    Ok(())
}

fn file_stem_str(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
}
