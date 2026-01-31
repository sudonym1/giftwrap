use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Protocol version for host <-> agent communication.
pub const INTERNAL_SPEC_VERSION: u32 = 1;

/// Container runtime inputs for a run invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub image: String,
    pub hostname: Option<String>,
    pub mounts: Vec<Mount>,
    pub env: BTreeMap<String, String>,
    pub workdir: Option<PathBuf>,
    pub user: Option<String>,
    pub extra_hosts: Vec<String>,
    pub privileged: bool,
    pub init: bool,
    pub remove: bool,
    pub interactive: bool,
    pub tty: bool,
    pub entrypoint: Option<Vec<String>>,
    pub command: Vec<String>,
    pub extra_args: Vec<String>,
}

/// Bind mount definition for the container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub source: PathBuf,
    pub target: PathBuf,
    pub read_only: bool,
    pub options: Vec<String>,
}

/// Spec passed to the in-container agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalSpec {
    pub protocol_version: u32,
    pub workdir: PathBuf,
    pub root_dir: PathBuf,
    pub user: UserSpec,
    pub env_overrides: BTreeMap<String, String>,
    pub persist_env: Option<PersistEnvSpec>,
    pub terminfo: Option<TerminfoSpec>,
    pub command: Vec<String>,
    pub shell: Option<String>,
    pub extra_shell: Option<PathBuf>,
    pub prefix_cmd: Vec<String>,
    pub prefix_cmd_quiet: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSpec {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistEnvSpec {
    pub path: PathBuf,
    pub restore: bool,
    pub save: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminfoSpec {
    pub term: String,
    pub data: Vec<u8>,
}
