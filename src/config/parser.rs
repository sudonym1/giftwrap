use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use super::Config;

/// Parse config file (TOML format)
pub fn parse_config(path: &Path) -> Result<Config> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    // Try TOML format first
    if path.extension().map_or(false, |ext| ext == "toml") {
        parse_toml_config(&content)
    } else {
        // For .giftwrap files, try whitespace-delimited format
        parse_whitespace_config(&content)
    }
}

fn parse_toml_config(content: &str) -> Result<Config> {
    let toml_config: toml::Value =
        toml::from_str(content).context("Failed to parse TOML config")?;

    let mut config = Config::default();

    // Parse container_image (required)
    if let Some(image) = toml_config.get("container_image") {
        config.container_image = image
            .as_str()
            .context("container_image must be a string")?
            .to_string();
    }

    // Parse optional fields
    if let Some(mount_to) = toml_config.get("mount_to") {
        config.mount_to = Some(
            mount_to
                .as_str()
                .context("mount_to must be a string")?
                .into(),
        );
    }

    if let Some(cd_to) = toml_config.get("cd_to") {
        config.cd_to = Some(cd_to.as_str().context("cd_to must be a string")?.into());
    }

    if let Some(extra_shares) = toml_config.get("extra_shares") {
        if let Some(array) = extra_shares.as_array() {
            for item in array {
                if let Some(s) = item.as_str() {
                    config.extra_shares.push(s.to_string());
                }
            }
        }
    }

    if let Some(extra_hosts) = toml_config.get("extra_hosts") {
        if let Some(array) = extra_hosts.as_array() {
            for item in array {
                if let Some(s) = item.as_str() {
                    config.extra_hosts.push(s.to_string());
                }
            }
        }
    }

    if let Some(env_overrides) = toml_config.get("env_overrides") {
        if let Some(array) = env_overrides.as_array() {
            for item in array {
                if let Some(s) = item.as_str() {
                    config.env_overrides.push(s.to_string());
                }
            }
        }
    }

    if let Some(prefix_cmd) = toml_config.get("prefix_cmd") {
        if let Some(array) = prefix_cmd.as_array() {
            let mut cmd = Vec::new();
            for item in array {
                if let Some(s) = item.as_str() {
                    cmd.push(s.to_string());
                }
            }
            config.prefix_cmd = Some(cmd);
        }
    }

    if let Some(prefix_cmd_quiet) = toml_config.get("prefix_cmd_quiet") {
        if let Some(array) = prefix_cmd_quiet.as_array() {
            let mut cmd = Vec::new();
            for item in array {
                if let Some(s) = item.as_str() {
                    cmd.push(s.to_string());
                }
            }
            config.prefix_cmd_quiet = Some(cmd);
        }
    }

    if let Some(version_by_build_context) = toml_config.get("version_by_build_context") {
        config.version_by_build_context = Some(
            version_by_build_context
                .as_str()
                .context("version_by_build_context must be a string")?
                .to_string(),
        );
    }

    if let Some(persist_environment) = toml_config.get("persist_environment") {
        config.persist_environment = Some(
            persist_environment
                .as_str()
                .context("persist_environment must be a string")?
                .into(),
        );
    }

    if let Some(prelaunch_hook) = toml_config.get("prelaunch_hook") {
        if let Some(array) = prelaunch_hook.as_array() {
            let mut hook = Vec::new();
            for item in array {
                if let Some(s) = item.as_str() {
                    hook.push(s.to_string());
                }
            }
            config.prelaunch_hook = Some(hook);
        }
    }

    if let Some(uuid) = toml_config.get("uuid") {
        config.uuid = Some(uuid.as_str().context("uuid must be a string")?.to_string());
    }

    if let Some(extra_args) = toml_config.get("extra_args") {
        if let Some(array) = extra_args.as_array() {
            for item in array {
                if let Some(s) = item.as_str() {
                    config.extra_args.push(s.to_string());
                }
            }
        }
    }

    if let Some(share_git_dir) = toml_config.get("share_git_dir") {
        config.share_git_dir = share_git_dir
            .as_bool()
            .context("share_git_dir must be a boolean")?;
    }

    if let Some(extra_shell) = toml_config.get("extra_shell") {
        config.extra_shell = Some(
            extra_shell
                .as_str()
                .context("extra_shell must be a string")?
                .into(),
        );
    }

    Ok(config)
}

fn parse_whitespace_config(content: &str) -> Result<Config> {
    let mut config = Config::default();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "container_image" => {
                if parts.len() >= 2 {
                    config.container_image = parts[1].to_string();
                }
            }
            "mount_to" => {
                if parts.len() >= 2 {
                    config.mount_to = Some(parts[1].into());
                }
            }
            "cd_to" => {
                if parts.len() >= 2 {
                    config.cd_to = Some(parts[1].into());
                }
            }
            "extra_shares" => {
                for share in parts.iter().skip(1) {
                    config.extra_shares.push(share.to_string());
                }
            }
            "extra_hosts" => {
                for host in parts.iter().skip(1) {
                    config.extra_hosts.push(host.to_string());
                }
            }
            "env_overrides" => {
                for env in parts.iter().skip(1) {
                    config.env_overrides.push(env.to_string());
                }
            }
            "prefix_cmd" => {
                config.prefix_cmd = Some(parts.iter().skip(1).map(|s| s.to_string()).collect());
            }
            "prefix_cmd_quiet" => {
                config.prefix_cmd_quiet =
                    Some(parts.iter().skip(1).map(|s| s.to_string()).collect());
            }
            "version_by_build_context" => {
                if parts.len() >= 2 {
                    config.version_by_build_context = Some(parts[1].to_string());
                }
            }
            "persist_environment" => {
                if parts.len() >= 2 {
                    config.persist_environment = Some(parts[1].into());
                }
            }
            "prelaunch_hook" => {
                config.prelaunch_hook = Some(parts.iter().skip(1).map(|s| s.to_string()).collect());
            }
            "uuid" => {
                if parts.len() >= 2 {
                    config.uuid = Some(parts[1].to_string());
                }
            }
            "extra_args" => {
                for arg in parts.iter().skip(1) {
                    config.extra_args.push(arg.to_string());
                }
            }
            "share_git_dir" => {
                if parts.len() >= 2 {
                    config.share_git_dir = parts[1]
                        .parse()
                        .context("Invalid boolean for share_git_dir")?;
                }
            }
            "extra_shell" => {
                if parts.len() >= 2 {
                    config.extra_shell = Some(parts[1].into());
                }
            }
            _ => {
                // Unknown parameter, skip for now
            }
        }
    }

    Ok(config)
}
