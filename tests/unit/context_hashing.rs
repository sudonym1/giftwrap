use std::fs;

use giftwrap::config;
use giftwrap::context_hash;

#[test]
fn context_hash_is_deterministic() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let config_dir = root.join(".giftwrap");
    fs::create_dir_all(&config_dir).expect("create config dir");

    fs::write(config_dir.join("setup.sh"), "#!/bin/sh\necho hi\n").expect("write setup");
    fs::write(
        config_dir.join("config.toml"),
        "image = \"docker.io/library/alpine:3\"\nsetup_script = \"setup.sh\"\n",
    )
    .expect("write config");

    let cfg = config::load(&config_dir.join("config.toml")).expect("load config");
    let a = context_hash::compute(root, &cfg).expect("hash a");
    let b = context_hash::compute(root, &cfg).expect("hash b");

    assert_eq!(a.ctx_sha, b.ctx_sha);
    assert_eq!(a.manifest_sha256, b.manifest_sha256);
    assert_eq!(a.manifest_entries, b.manifest_entries);
}

#[test]
fn context_hash_changes_when_setup_changes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let config_dir = root.join(".giftwrap");
    fs::create_dir_all(&config_dir).expect("create config dir");

    fs::write(config_dir.join("setup.sh"), "#!/bin/sh\necho one\n").expect("write setup");
    fs::write(
        config_dir.join("config.toml"),
        "image = \"docker.io/library/alpine:3\"\nsetup_script = \"setup.sh\"\n",
    )
    .expect("write config");

    let cfg = config::load(&config_dir.join("config.toml")).expect("load config");
    let before = context_hash::compute(root, &cfg).expect("hash before");

    fs::write(config_dir.join("setup.sh"), "#!/bin/sh\necho two\n").expect("update setup");
    let after = context_hash::compute(root, &cfg).expect("hash after");

    assert_ne!(before.ctx_sha, after.ctx_sha);
}

#[test]
fn context_hash_does_not_change_when_env_changes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let config_dir = root.join(".giftwrap");
    fs::create_dir_all(&config_dir).expect("create config dir");

    fs::write(config_dir.join("setup.sh"), "#!/bin/sh\necho one\n").expect("write setup");
    fs::write(
        config_dir.join("config.toml"),
        "image = \"docker.io/library/alpine:3\"\nsetup_script = \"setup.sh\"\n[env]\nPATH = \"/usr/bin\"\n",
    )
    .expect("write config");

    let cfg = config::load(&config_dir.join("config.toml")).expect("load config");
    let before = context_hash::compute(root, &cfg).expect("hash before");

    fs::write(
        config_dir.join("config.toml"),
        "image = \"docker.io/library/alpine:3\"\nsetup_script = \"setup.sh\"\n[env]\nPATH = \"/usr/local/bin:/usr/bin\"\n",
    )
    .expect("update config");

    let cfg = config::load(&config_dir.join("config.toml")).expect("reload config");
    let after = context_hash::compute(root, &cfg).expect("hash after");

    assert_eq!(before.ctx_sha, after.ctx_sha);
}
