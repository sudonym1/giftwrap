use std::collections::BTreeMap;
use std::fs;

use giftwrap::sqfs_cache::{self, CacheMetadata, PullPolicy};

#[test]
fn valid_cache_metadata_is_accepted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = sqfs_cache::resolve_paths(tmp.path(), "abc123");
    sqfs_cache::ensure_layout(&paths).expect("layout");

    fs::write(&paths.sqfs, b"sqfs").expect("write sqfs");
    let metadata = CacheMetadata {
        schema_version: 1,
        ctx_sha: "abc123".to_string(),
        image_ref: "docker.io/library/alpine:3".to_string(),
        image_digest: "sha256:123".to_string(),
        pull_policy_used: "missing".to_string(),
        setup_script_sha256: "deadbeef".to_string(),
        context_manifest_sha256: "feedface".to_string(),
        compression: "zstd".to_string(),
        created_unix_ms: 0,
        giftwrap_version: "2.0.0".to_string(),
        tool_versions: BTreeMap::new(),
    };
    fs::write(
        &paths.meta,
        serde_json::to_vec(&metadata).expect("metadata json"),
    )
    .expect("write metadata");

    let valid = sqfs_cache::is_valid(&paths, PullPolicy::Missing).expect("valid metadata");
    assert!(valid);
}

#[test]
fn invalid_compression_is_rejected() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = sqfs_cache::resolve_paths(tmp.path(), "abc123");
    sqfs_cache::ensure_layout(&paths).expect("layout");

    fs::write(&paths.sqfs, b"sqfs").expect("write sqfs");
    let mut metadata = CacheMetadata {
        schema_version: 1,
        ctx_sha: "abc123".to_string(),
        image_ref: "docker.io/library/alpine:3".to_string(),
        image_digest: "sha256:123".to_string(),
        pull_policy_used: "missing".to_string(),
        setup_script_sha256: "deadbeef".to_string(),
        context_manifest_sha256: "feedface".to_string(),
        compression: "lzo".to_string(),
        created_unix_ms: 0,
        giftwrap_version: "2.0.0".to_string(),
        tool_versions: BTreeMap::new(),
    };

    fs::write(
        &paths.meta,
        serde_json::to_vec(&metadata).expect("metadata json"),
    )
    .expect("write metadata");

    let err = sqfs_cache::is_valid(&paths, PullPolicy::Missing)
        .expect_err("invalid compression should fail validation");
    assert!(err.to_string().contains("unsupported cache compression"));

    metadata.compression = "zstd".to_string();
    metadata.ctx_sha = "mismatch".to_string();
    fs::write(
        &paths.meta,
        serde_json::to_vec(&metadata).expect("metadata json"),
    )
    .expect("write metadata");

    let err = sqfs_cache::is_valid(&paths, PullPolicy::Missing)
        .expect_err("ctx mismatch should fail validation");
    assert!(err.to_string().contains("ctx_sha mismatch"));
}

#[test]
fn pull_policy_controls_validity_behavior() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = sqfs_cache::resolve_paths(tmp.path(), "abc123");

    let always =
        sqfs_cache::is_valid(&paths, PullPolicy::Always).expect("always policy should bypass");
    assert!(!always);

    let never_err = sqfs_cache::is_valid(&paths, PullPolicy::Never)
        .expect_err("never policy should fail if cache is missing");
    assert!(never_err.to_string().contains("pull policy 'never'"));
}
