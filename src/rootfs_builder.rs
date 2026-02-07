use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::errors::GiftwrapError;
use crate::log::Logger;
use crate::process::{self, CommandFailureKind};

const SETUP_SCRIPT_DEST: &str = "/tmp/giftwrap-setup.sh";

pub fn unpack(
    layout_dir: &Path,
    bundle_dir: &Path,
    logger: &Logger,
) -> Result<PathBuf, GiftwrapError> {
    fs::create_dir_all(bundle_dir).map_err(|err| {
        GiftwrapError::build(format!(
            "failed to create bundle directory {}: {err}",
            bundle_dir.display()
        ))
    })?;

    let args = unpack_command_args(layout_dir, bundle_dir);
    process::run_checked(
        "umoci",
        &args,
        CommandFailureKind::Build,
        "unpack OCI image",
        logger,
    )?;

    let rootfs = bundle_dir.join("rootfs");
    if !rootfs.is_dir() {
        return Err(GiftwrapError::build(format!(
            "failed to unpack OCI image (missing rootfs at {})",
            rootfs.display()
        )));
    }

    Ok(rootfs)
}

pub fn run_setup(
    rootfs: &Path,
    cfg: &Config,
    ctx_sha: &str,
    build_root: &Path,
    cache_dir: &Path,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    let setup_source = cfg.resolve_setup_script(build_root);
    let setup_host_target = rootfs.join("tmp/giftwrap-setup.sh");

    if let Some(parent) = setup_host_target.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            GiftwrapError::build(format!(
                "failed to prepare setup script destination {}: {err}",
                parent.display()
            ))
        })?;
    }

    fs::copy(&setup_source, &setup_host_target).map_err(|err| {
        GiftwrapError::build(format!(
            "failed to copy setup script to bundle {}: {err}",
            setup_host_target.display()
        ))
    })?;

    let mut perms = fs::metadata(&setup_host_target)
        .map_err(|err| {
            GiftwrapError::build(format!(
                "failed to stat copied setup script {}: {err}",
                setup_host_target.display()
            ))
        })?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&setup_host_target, perms).map_err(|err| {
        GiftwrapError::build(format!(
            "failed to set execute bit on setup script {}: {err}",
            setup_host_target.display()
        ))
    })?;

    let args = setup_bwrap_args(rootfs);
    logger.command("bwrap", &args);

    let mut command = Command::new("bwrap");
    command
        .args(&args)
        .env("GW_BUILD_ROOT", build_root)
        .env("GW_CTX_SHA", ctx_sha)
        .env("GW_IMAGE_REF", &cfg.image)
        .env("GW_CACHE_DIR", cache_dir)
        // Avoid leaking host HOME into root sandboxed setup processes.
        .env("HOME", "/root")
        .env("USER", "root")
        .env("LOGNAME", "root")
        // Rootless user namespaces cannot chown arbitrary IDs during extraction.
        .env("TAR_OPTIONS", "--no-same-owner --no-same-permissions");

    if let Ok(term) = std::env::var("TERM") {
        command.env("TERM", term);
    }

    let status = command.status().map_err(|err| {
        GiftwrapError::build_hint(
            format!("failed to execute setup script in bwrap: {err}"),
            "ensure bwrap is installed and unprivileged user namespaces are enabled",
        )
    })?;

    if status.success() {
        return Ok(());
    }

    Err(GiftwrapError::build_hint(
        format!(
            "failed to execute setup script ({})",
            crate::errors::exit_status_desc(&status)
        ),
        "ensure the setup script is idempotent and host user namespaces are available",
    ))
}

pub fn build_sqfs(rootfs: &Path, output_tmp: &Path, logger: &Logger) -> Result<(), GiftwrapError> {
    let args = mksquashfs_command_args(rootfs, output_tmp);
    process::run_checked(
        "mksquashfs",
        &args,
        CommandFailureKind::Build,
        "build squashfs",
        logger,
    )
}

pub fn unpack_command_args(layout_dir: &Path, bundle_dir: &Path) -> Vec<String> {
    vec![
        "unpack".to_string(),
        "--rootless".to_string(),
        "--image".to_string(),
        format!("{}:base", layout_dir.display()),
        bundle_dir.display().to_string(),
    ]
}

pub fn setup_bwrap_args(rootfs: &Path) -> Vec<String> {
    vec![
        "--die-with-parent".to_string(),
        "--new-session".to_string(),
        "--unshare-all".to_string(),
        "--share-net".to_string(),
        "--uid".to_string(),
        "0".to_string(),
        "--gid".to_string(),
        "0".to_string(),
        "--bind".to_string(),
        rootfs.display().to_string(),
        "/".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--chdir".to_string(),
        "/".to_string(),
        "--dir".to_string(),
        "/run/systemd".to_string(),
        "--dir".to_string(),
        "/run/systemd/resolve".to_string(),
        "--ro-bind".to_string(),
        "/etc/resolv.conf".to_string(),
        "/etc/resolv.conf".to_string(),
        "--ro-bind".to_string(),
        "/etc/resolv.conf".to_string(),
        "/run/systemd/resolve/stub-resolv.conf".to_string(),
        "--ro-bind".to_string(),
        "/etc/resolv.conf".to_string(),
        "/run/systemd/resolve/resolv.conf".to_string(),
        "/bin/sh".to_string(),
        SETUP_SCRIPT_DEST.to_string(),
    ]
}

pub fn mksquashfs_command_args(rootfs: &Path, output_tmp: &Path) -> Vec<String> {
    vec![
        rootfs.display().to_string(),
        output_tmp.display().to_string(),
        "-comp".to_string(),
        "zstd".to_string(),
        "-xattrs".to_string(),
        "-noappend".to_string(),
    ]
}
