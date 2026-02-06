use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::discovery::CONFIG_FILENAME;
use crate::errors::GiftwrapError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub relative_path: String,
    pub file_sha256: String,
    pub mode: u32,
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextHashResult {
    pub ctx_sha: String,
    pub manifest_sha256: String,
    pub manifest_entries: Vec<ManifestEntry>,
    pub setup_script_sha256: String,
}

pub fn compute(build_root: &Path, cfg: &Config) -> Result<ContextHashResult, GiftwrapError> {
    let config_path = build_root.join(CONFIG_FILENAME);
    let config_bytes = fs::read(&config_path).map_err(|err| {
        GiftwrapError::config(format!(
            "failed to read config for hashing {}: {err}",
            config_path.display()
        ))
    })?;

    let setup_script_path = cfg.resolve_setup_script(build_root);
    let mut entries_by_path = BTreeMap::new();

    let config_entry = manifest_entry_for_path(build_root, &config_path)?;
    entries_by_path.insert(config_entry.relative_path.clone(), config_entry);

    let setup_entry = manifest_entry_for_path(build_root, &setup_script_path)?;
    let setup_script_sha256 = setup_entry.file_sha256.clone();
    entries_by_path.insert(setup_entry.relative_path.clone(), setup_entry);

    let manifest_entries = entries_by_path.into_values().collect::<Vec<_>>();
    let manifest_bytes = encode_manifest(&manifest_entries);
    let manifest_sha256 = hash_bytes(&manifest_bytes);

    let mut ctx_hasher = Sha256::new();
    ctx_hasher.update(b"giftwrap-v2\0");
    ctx_hasher.update(&config_bytes);
    ctx_hasher.update(b"\0");
    ctx_hasher.update(cfg.image.as_bytes());
    ctx_hasher.update(b"\0");
    ctx_hasher.update(setup_script_sha256.as_bytes());
    ctx_hasher.update(b"\0");
    ctx_hasher.update(&manifest_bytes);
    ctx_hasher.update(b"\0");

    let ctx_sha = hex::encode(ctx_hasher.finalize());

    Ok(ContextHashResult {
        ctx_sha,
        manifest_sha256,
        manifest_entries,
        setup_script_sha256,
    })
}

fn manifest_entry_for_path(build_root: &Path, path: &Path) -> Result<ManifestEntry, GiftwrapError> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        GiftwrapError::config(format!(
            "failed to stat context path {}: {err}",
            path.display()
        ))
    })?;

    let mode = metadata.permissions().mode();
    let relative_path = to_manifest_path(build_root, path);

    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|err| {
            GiftwrapError::config(format!("failed to read symlink {}: {err}", path.display()))
        })?;
        let target_str = target.to_string_lossy().to_string();

        let mut hasher = Sha256::new();
        hasher.update(b"symlink\0");
        hasher.update(target_str.as_bytes());
        hasher.update(b"\0");
        hasher.update(mode.to_string().as_bytes());
        let file_sha256 = hex::encode(hasher.finalize());

        return Ok(ManifestEntry {
            relative_path,
            file_sha256,
            mode,
            symlink_target: Some(target_str),
        });
    }

    let bytes = fs::read(path).map_err(|err| {
        GiftwrapError::config(format!(
            "failed to read context path {}: {err}",
            path.display()
        ))
    })?;

    Ok(ManifestEntry {
        relative_path,
        file_sha256: hash_bytes(&bytes),
        mode,
        symlink_target: None,
    })
}

fn encode_manifest(entries: &[ManifestEntry]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entry in entries {
        bytes.extend_from_slice(entry.relative_path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(entry.file_sha256.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(entry.mode.to_string().as_bytes());
        bytes.push(0);
        if let Some(target) = &entry.symlink_target {
            bytes.extend_from_slice(target.as_bytes());
        }
        bytes.push(0);
    }
    bytes
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn to_manifest_path(build_root: &Path, path: &Path) -> String {
    if path == build_root.join(CONFIG_FILENAME) {
        return CONFIG_FILENAME.to_string();
    }

    if let Ok(relative) = path.strip_prefix(build_root) {
        return pathbuf_to_manifest_string(relative);
    }

    pathbuf_to_manifest_string(&PathBuf::from(path))
}

fn pathbuf_to_manifest_string(path: &Path) -> String {
    let parts = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}
