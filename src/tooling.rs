use std::collections::BTreeMap;
use std::process::Command;

use which::which;

use crate::errors::GiftwrapError;
use crate::log::Logger;

#[derive(Debug, Clone)]
pub struct ProbedTools {
    pub tool_versions: BTreeMap<String, String>,
    pub unmount_tool: String,
}

pub fn probe_required(logger: &Logger) -> Result<ProbedTools, GiftwrapError> {
    let required = [
        "bwrap",
        "skopeo",
        "umoci",
        "mksquashfs",
        "squashfuse",
        "fuse-overlayfs",
    ];
    let mut tool_versions = BTreeMap::new();

    for tool in required {
        which(tool)
            .map_err(|_| GiftwrapError::tooling(format!("required tool not found: {tool}")))?;
        let version = detect_version(tool);
        tool_versions.insert(tool.to_string(), version);
    }

    let unmount_tool = if which("fusermount3").is_ok() {
        "fusermount3".to_string()
    } else if which("umount").is_ok() {
        "umount".to_string()
    } else {
        return Err(GiftwrapError::tooling(
            "required tool not found: fusermount3 (or umount fallback)",
        ));
    };

    tool_versions.insert(unmount_tool.clone(), detect_version(&unmount_tool));

    if logger.verbose() {
        let rendered = tool_versions
            .iter()
            .map(|(tool, version)| format!("{tool}={version}"))
            .collect::<Vec<_>>()
            .join(", ");
        logger.event(format!("detected tool versions: {rendered}"));
    }

    Ok(ProbedTools {
        tool_versions,
        unmount_tool,
    })
}

fn detect_version(tool: &str) -> String {
    for arg in ["--version", "-V", "version"] {
        if let Ok(output) = Command::new(tool).arg(arg).output() {
            if output.status.success() {
                if let Some(line) = std::str::from_utf8(&output.stdout)
                    .ok()
                    .and_then(|stdout| stdout.lines().next())
                {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }

                if let Some(line) = std::str::from_utf8(&output.stderr)
                    .ok()
                    .and_then(|stderr| stderr.lines().next())
                {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
        }
    }

    "unknown".to_string()
}
