# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Giftwrap is a container wrapper tool that seamlessly integrates containerized build environments with the host filesystem and user context. It allows developers to run commands inside containers as if they were running natively, while maintaining proper file permissions, environment variables, and terminal capabilities.

This is a Rust implementation inspired by the Python script at `inspiration/docker-runner.py`. The design is documented in `DESIGN.md`.

**Current Status**: Early development - Rust project not yet initialized. The Rust implementation uses **Podman** as the container runtime (Podman only - no other runtimes supported).

## Build Commands

This project will be a standard Rust binary. Once initialized with `cargo init`, use:

```bash
# Build the project
cargo build

# Build for release
cargo build --release

# Run tests
cargo test

# Run a specific test
cargo test test_name

# Run with cargo
cargo run -- [giftwrap args]

# Check code without building
cargo check

# Format code
cargo fmt

# Run lints
cargo clippy
```

## Architecture

### Module Structure (Planned)

Based on `DESIGN.md`, the codebase should be organized as:

```
src/
├── main.rs              # CLI argument parsing and orchestration
├── config/              # Configuration file parsing and environment variable overrides
├── build_context/       # SHA computation for build context versioning
├── podman/              # Podman command generation and execution
├── terminal/            # TTY detection and terminfo handling
├── bootstrap/           # Bootstrap code generation for containers
└── git.rs               # Git directory detection for worktrees/submodules
```

### Key Components

**Configuration System**:
- Searches up directory tree from CWD for config file (`.giftwrap`, `giftwrap.toml`, etc. - TBD)
- Parses config file (format TBD - Python version uses whitespace-delimited, Rust may use TOML)
- Applies environment variable overrides with `GW_USER_OPT_SET_*`, `GW_USER_OPT_ADD_*`, `GW_USER_OPT_DEL_*` prefixes
- UUID-scoped overrides: `GW_USER_OPT_*_UUID_<uuid>_<param>` for config-specific modifications

**Build Context Versioning**:
- Reads `.giftwrapped` file (gitignore-style include patterns)
- Computes SHA1/SHA256 hash of all included files
- Caches result in shafile with file list for dirty checking
- Tags container images with context SHA for reproducibility

**Container Runtime**:
- **Podman only** (daemonless, fast, Docker-compatible CLI)
- No support for other runtimes - Podman is required

**Bootstrap Mechanism**:
- Generates code that runs inside container to set up environment
- Creates user/group matching host UID/GID for proper file permissions
- Restores persistent environment if configured
- Drops privileges before executing user command
- Installs terminfo for proper terminal emulation

**Terminal Integration**:
- Detects TTY on stdin/stdout
- Extracts terminfo via `infocmp $TERM` on host
- Base64-encodes and passes to container
- Installs via `tic` in container's user home directory

**Command Execution Flow**:
1. Find and parse config file
2. Apply environment variable overrides
3. Verify Podman is available
4. Compute build context SHA (if `version_by_build_context` enabled)
5. Build Podman command with mounts, privileges, environment
6. Generate bootstrap script
7. Execute container (or print command if `--gw-print`)

### Critical Design Patterns

**User Creation Inside Container**:
- All file operations on mounted volumes must use correct UID/GID
- Bootstrap code dynamically creates matching user/group
- Commands run as created user (not root) for security

**Environment Persistence**:
- Optional feature via `persist_environment` config parameter
- Saves environment to file after user command completes
- Restores on next invocation for stateful development sessions
- Allows `source venv/bin/activate` and similar commands to persist

**Git Directory Sharing**:
- Git worktrees and submodules store `.git` outside repository
- If `share_git_dir` is set, runs `git rev-parse --git-common-dir`
- Mounts git directory if outside build root
- Ensures git commands work correctly in container

## Implementation Considerations

### Container Runtime

**Podman Only**

Giftwrap requires Podman and does not support other container runtimes.

Why Podman:
- **Daemonless architecture**: No background service required, simpler deployment
- **Fast startup**: Can achieve <50ms container startup for cached images
- **Docker-compatible CLI**: Minimal migration effort from Python script
- **Modern architecture**: Better security model, rootless support
- **Image building**: Uses buildah integration for `version_by_build_context` feature

Giftwrap will error at startup if Podman is not available.


### Configuration File Format

TBD - Options:
- TOML (Rust ecosystem standard)
- Simple whitespace-delimited (Python version compatibility)
- YAML (user-friendly but adds dependency)

### Bootstrap Implementation

Options for running setup code inside container:
- Shell script (portable, simple)
- Small Rust binary helper (type-safe, bundled)
- Python script (Python version approach, requires Python in container)

### CLI Flag Prefix

Design doc uses `--gw-` prefix for giftwrap-specific flags:
- `--gw-print`: Print container command instead of executing
- `--gw-ctx`: Print build context SHA
- `--gw-print-image`: Print image name with SHA
- `--gw-use-ctx=SHA`: Force specific context SHA
- `--gw-img=IMAGE`: Override container image
- `--gw-rebuild`: Rebuild container before running
- `--gw-extra-args=ARGS`: Pass extra args to container runtime
- `--gw-show-config`: Dump parsed configuration
- `--gw-help`: Show help

Argument separator: `--` splits giftwrap/runtime args from user command

## Key Data Structures (Planned)

```rust
struct Config {
    container_image: String,
    mount_to: Option<PathBuf>,
    cd_to: Option<PathBuf>,
    extra_shares: Vec<String>,
    extra_hosts: Vec<String>,
    env_overrides: Vec<String>,
    version_by_build_context: Option<PathBuf>,  // shafile path
    persist_environment: Option<PathBuf>,
    prelaunch_hook: Option<Vec<String>>,
    uuid: Option<String>,
    // ... other fields per DESIGN.md
}

struct BuildContext {
    files: Vec<PathBuf>,
    sha: String,
}

struct UserContext {
    username: String,
    uid: u32,
    gid: u32,
    cwd: PathBuf,
}

// Podman command builder
struct PodmanCommand {
    // Methods to build podman run/build commands
}
```

## Python Script Reference

The inspiration script (`inspiration/docker-runner.py`) demonstrates:
- Build root discovery by walking up directories
- Simple whitespace-delimited config parsing
- Environment variable override system with `DR_USER_OPT_*` prefixes
- Build context SHA computation with dirty checking
- Bootstrap code injection via Python `-c` flag
- TTY detection and terminfo handling
- Privilege dropping with matching UID/GID
- Environment persistence using pickle

Key differences in Rust version:
- Use `GW_` prefix instead of `DR_` for environment variables
- Uses Podman instead of Docker (Podman required, no other runtimes supported)
- Modern config format (likely TOML)
- Type-safe implementation
