use std::fs::OpenOptions;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use fs4::fs_std::FileExt;

use giftwrap::sqfs_cache;

#[test]
fn lock_times_out_when_held_elsewhere() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = sqfs_cache::resolve_paths(tmp.path(), "abc123");
    sqfs_cache::ensure_layout(&paths).expect("layout");

    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.lock)
        .expect("open lock file");
    lock_file.lock_exclusive().expect("lock should succeed");

    let err = sqfs_cache::with_lock(&paths, Duration::from_millis(150), || {
        Ok::<_, giftwrap::errors::GiftwrapError>(())
    })
    .expect_err("should time out when lock is held");

    assert!(err.to_string().contains("cache lock timeout"));
    lock_file.unlock().expect("unlock");
}

#[test]
fn lock_becomes_available_after_holder_exits() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = sqfs_cache::resolve_paths(tmp.path(), "abc123");
    sqfs_cache::ensure_layout(&paths).expect("layout");

    let (tx, rx) = mpsc::channel();
    let thread_paths = paths.clone();
    let holder = thread::spawn(move || {
        sqfs_cache::with_lock(&thread_paths, Duration::from_secs(2), || {
            tx.send(()).expect("signal lock acquired");
            thread::sleep(Duration::from_millis(250));
            Ok::<_, giftwrap::errors::GiftwrapError>(())
        })
        .expect("holder lock should succeed");
    });

    rx.recv().expect("wait for holder");

    let timed_out = sqfs_cache::with_lock(&paths, Duration::from_millis(80), || {
        Ok::<_, giftwrap::errors::GiftwrapError>(())
    });
    assert!(
        timed_out.is_err(),
        "second locker should time out while held"
    );

    holder.join().expect("holder thread should join");

    sqfs_cache::with_lock(&paths, Duration::from_millis(500), || {
        Ok::<_, giftwrap::errors::GiftwrapError>(())
    })
    .expect("lock should succeed after holder releases");
}
