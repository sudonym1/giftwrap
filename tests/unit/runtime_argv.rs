use std::collections::BTreeMap;
use std::path::PathBuf;

use giftwrap::runtime::bwrap::{self, RunSpec};

#[test]
fn runtime_bwrap_argv_contains_required_defaults() {
    let mut env = BTreeMap::new();
    env.insert("HOME".to_string(), "/home/tester".to_string());
    env.insert("PATH".to_string(), "/usr/local/bin:/usr/bin".to_string());

    let spec = RunSpec {
        host_uid: 1000,
        host_gid: 1000,
        build_root: PathBuf::from("/workspace/project"),
        workdir: PathBuf::from("/workspace/project/src"),
        mountpoint: PathBuf::from("/tmp/cache/mnt/ctx"),
        env,
        argv: vec!["bash".to_string(), "-lc".to_string(), "echo hi".to_string()],
    };

    let argv = bwrap::build_argv(&spec);

    for required in [
        "--die-with-parent",
        "--new-session",
        "--unshare-ipc",
        "--unshare-pid",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--share-net",
        "--unshare-user-try",
        "--clearenv",
    ] {
        assert!(
            argv.contains(&required.to_string()),
            "missing arg {required}"
        );
    }

    assert!(argv.windows(2).any(|window| window == ["--uid", "1000"]));
    assert!(argv.windows(2).any(|window| window == ["--gid", "1000"]));
    assert!(argv
        .windows(3)
        .any(|window| window == ["--overlay-src", "/tmp/cache/mnt/ctx", "--tmp-overlay"]));

    let dash = argv
        .iter()
        .position(|item| item == "--")
        .expect("separator");
    assert_eq!(&argv[dash + 1..], &["bash", "-lc", "echo hi"]);
}
