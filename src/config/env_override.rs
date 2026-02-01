use anyhow::{Context, Result};
use std::collections::HashMap;
use std::env;
use std::path::Path;

use super::Config;

/// Apply environment variable overrides to config
pub fn apply_env_overrides(config: &mut Config) -> Result<()> {
    // Collect all environment variables starting with GW_USER_OPT_
    let env_vars: HashMap<String, String> = env::vars()
        .filter(|(k, _)| k.starts_with("GW_USER_OPT_"))
        .collect();

    // Process SET operations
    for (key, value) in env_vars.iter() {
        if key.starts_with("GW_USER_OPT_SET_") {
            let param_name = key
                .strip_prefix("GW_USER_OPT_SET_")
                .context("Invalid SET env var format")?;

            // Handle UUID-scoped overrides - check if this looks like a UUID (has hyphens and reasonable length)
            if let Some((uuid, param)) = param_name.split_once('_') {
                if uuid.contains('-') && uuid.len() > 20 {
                    // This looks like a UUID-scoped override
                    if let Some(config_uuid) = &config.uuid {
                        if uuid == config_uuid {
                            apply_set_override(config, param, value)?;
                        }
                    }
                } else {
                    // This contains underscores but isn't UUID-scoped (e.g., "extra_args")
                    apply_set_override(config, param_name, value)?;
                }
            } else {
                // No underscores, apply directly
                apply_set_override(config, param_name, value)?;
            }
        }
    }

    // Process ADD operations
    for (key, value) in env_vars.iter() {
        if key.starts_with("GW_USER_OPT_ADD_") {
            let param_name = key
                .strip_prefix("GW_USER_OPT_ADD_")
                .context("Invalid ADD env var format")?;

            // Handle UUID-scoped overrides - check if this looks like a UUID
            if let Some((uuid, param)) = param_name.split_once('_') {
                if uuid.contains('-') && uuid.len() > 20 {
                    // This looks like a UUID-scoped override
                    if let Some(config_uuid) = &config.uuid {
                        if uuid == config_uuid {
                            apply_add_override(config, param, value)?;
                        }
                    }
                } else {
                    // This contains underscores but isn't UUID-scoped
                    apply_add_override(config, param_name, value)?;
                }
            } else {
                // No underscores, apply directly
                apply_add_override(config, param_name, value)?;
            }
        }
    }

    // Process DEL operations
    for (key, value) in env_vars.iter() {
        if key.starts_with("GW_USER_OPT_DEL_") {
            let param_name = key
                .strip_prefix("GW_USER_OPT_DEL_")
                .context("Invalid DEL env var format")?;

            // Handle UUID-scoped overrides - check if this looks like a UUID
            if let Some((uuid, param)) = param_name.split_once('_') {
                if uuid.contains('-') && uuid.len() > 20 {
                    // This looks like a UUID-scoped override
                    if let Some(config_uuid) = &config.uuid {
                        if uuid == config_uuid {
                            apply_del_override(config, param, value)?;
                        }
                    }
                } else {
                    // This contains underscores but isn't UUID-scoped
                    apply_del_override(config, param_name, value)?;
                }
            } else {
                // No underscores, apply directly
                apply_del_override(config, param_name, value)?;
            }
        }
    }

    Ok(())
}

fn apply_set_override(config: &mut Config, param: &str, value: &str) -> Result<()> {
    match param {
        "container_image" => config.container_image = value.to_string(),
        "mount_to" => config.mount_to = Some(Path::new(value).to_path_buf()),
        "cd_to" => config.cd_to = Some(Path::new(value).to_path_buf()),
        "version_by_build_context" => config.version_by_build_context = Some(value.to_string()),
        "persist_environment" => config.persist_environment = Some(Path::new(value).to_path_buf()),
        "uuid" => config.uuid = Some(value.to_string()),
        "share_git_dir" => {
            config.share_git_dir = value.parse().context("Invalid boolean for share_git_dir")?
        }
        "extra_shell" => config.extra_shell = Some(Path::new(value).to_path_buf()),
        _ => anyhow::bail!("Unknown SET parameter: {}", param),
    }
    Ok(())
}

fn apply_add_override(config: &mut Config, param: &str, value: &str) -> Result<()> {
    match param {
        "extra_shares" => config.extra_shares.push(value.to_string()),
        "extra_hosts" => config.extra_hosts.push(value.to_string()),
        "env_overrides" => config.env_overrides.push(value.to_string()),
        "extra_args" => config.extra_args.push(value.to_string()),
        _ => anyhow::bail!("Unknown ADD parameter: {}", param),
    }
    Ok(())
}

fn apply_del_override(config: &mut Config, param: &str, value: &str) -> Result<()> {
    match param {
        "extra_shares" => config.extra_shares.retain(|v| v != value),
        "extra_hosts" => config.extra_hosts.retain(|v| v != value),
        "env_overrides" => config.env_overrides.retain(|v| v != value),
        "extra_args" => config.extra_args.retain(|v| v != value),
        _ => anyhow::bail!("Unknown DEL parameter: {}", param),
    }
    Ok(())
}
