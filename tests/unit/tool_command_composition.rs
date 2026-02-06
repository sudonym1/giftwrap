use std::path::Path;

use giftwrap::oci;
use giftwrap::rootfs_builder;

#[test]
fn skopeo_command_args_are_composed_correctly() {
    let pull = oci::pull_command_args(
        "docker.io/library/debian:bookworm-slim",
        Path::new("/tmp/work/oci"),
    );
    assert_eq!(
        pull,
        vec![
            "copy",
            "docker://docker.io/library/debian:bookworm-slim",
            "oci:/tmp/work/oci:base",
        ]
    );

    let inspect = oci::inspect_command_args("docker.io/library/debian:bookworm-slim");
    assert_eq!(
        inspect,
        vec!["inspect", "docker://docker.io/library/debian:bookworm-slim"]
    );
}

#[test]
fn umoci_and_mksquashfs_args_are_composed_correctly() {
    let unpack = rootfs_builder::unpack_command_args(
        Path::new("/tmp/work/oci"),
        Path::new("/tmp/work/bundle"),
    );
    assert_eq!(
        unpack,
        vec![
            "unpack",
            "--rootless",
            "--image",
            "/tmp/work/oci:base",
            "/tmp/work/bundle",
        ]
    );

    let mk = rootfs_builder::mksquashfs_command_args(
        Path::new("/tmp/work/bundle/rootfs"),
        Path::new("/tmp/cache/ctx.sqfs.tmp"),
    );
    assert_eq!(
        mk,
        vec![
            "/tmp/work/bundle/rootfs",
            "/tmp/cache/ctx.sqfs.tmp",
            "-comp",
            "zstd",
            "-xattrs",
            "-noappend",
        ]
    );
}
