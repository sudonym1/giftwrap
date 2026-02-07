use std::fs;

use giftwrap::log::Logger;
use giftwrap::{runtime, sqfs_cache};

#[test]
fn reset_all_overlays_removes_single_and_legacy_overlay_directories() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let build_root = tmp.path().join("project");
    let state_root = build_root.join(".giftwrap");
    fs::create_dir_all(&state_root).expect("create state root");

    fs::write(state_root.join("config.toml"), "image = \"alpine\"\n").expect("write config");
    fs::write(state_root.join("context"), "ctx\n").expect("write context marker");

    let overlay = state_root.join("overlay");
    fs::create_dir_all(overlay.join("upper")).expect("create single overlay upper");
    fs::create_dir_all(overlay.join("work")).expect("create single overlay work");

    let ctx_a = state_root.join("a".repeat(64));
    let ctx_b = state_root.join("b".repeat(64));
    fs::create_dir_all(ctx_a.join("upper")).expect("create ctx_a upper");
    fs::create_dir_all(ctx_a.join("work")).expect("create ctx_a work");
    fs::create_dir_all(ctx_b.join("upper")).expect("create ctx_b upper");
    fs::create_dir_all(ctx_b.join("work")).expect("create ctx_b work");

    let unrelated = state_root.join("notes");
    fs::create_dir_all(&unrelated).expect("create unrelated dir");

    let logger = Logger::new(false);
    let removed = runtime::reset_all_overlays(&build_root, &logger).expect("reset overlays");
    assert_eq!(removed, 3);

    assert!(!overlay.exists());
    assert!(!ctx_a.exists());
    assert!(!ctx_b.exists());
    assert!(state_root.join("config.toml").exists());
    assert!(state_root.join("context").exists());
    assert!(unrelated.exists());
}

#[test]
fn reset_all_cache_entries_clears_cache_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_root = tmp.path().join("cache");
    fs::create_dir_all(&cache_root).expect("create cache root");

    let paths = sqfs_cache::resolve_paths(&cache_root, "abc123");
    sqfs_cache::ensure_layout(&paths).expect("create cache layout");
    fs::write(&paths.sqfs, b"sqfs").expect("write sqfs");
    fs::write(&paths.meta, b"{}").expect("write metadata");
    fs::write(&paths.lock, b"").expect("write lock");
    fs::create_dir_all(paths.work_root.join("abc123-1-1")).expect("create work dir");
    fs::create_dir_all(&paths.mountpoint).expect("create mountpoint");

    let removed = sqfs_cache::reset_all(&cache_root).expect("reset cache root");
    assert!(removed > 0);

    let mut entries = fs::read_dir(&cache_root).expect("read cache root");
    assert!(entries.next().is_none(), "cache root should be empty");
}

#[test]
fn reset_all_cache_entries_does_not_touch_project_overlays() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_root = tmp.path().join("cache");
    fs::create_dir_all(&cache_root).expect("create cache root");
    fs::write(cache_root.join("foo.sqfs"), b"sqfs").expect("write cache artifact");

    let build_root = tmp.path().join("project");
    let overlay = build_root
        .join(".giftwrap")
        .join("a".repeat(64))
        .join("upper");
    fs::create_dir_all(&overlay).expect("create overlay dir");

    sqfs_cache::reset_all(&cache_root).expect("reset cache root");

    assert!(
        overlay.exists(),
        "cache reset should not delete project overlay directories"
    );
}
