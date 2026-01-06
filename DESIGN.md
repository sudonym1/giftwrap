# Giftwrap Design Document

## Overview

Giftwrap is a container wrapper tool that seamlessly integrates containerized
build environments with the host filesystem and user context. It allows
developers to run commands inside containers as if they were running natively,
while maintaining proper file permissions, environment variables, and terminal
capabilities.

**Note**: The Python inspiration script (`inspiration/docker-runner.py`) uses
Docker specifically. The Rust implementation uses **Podman** as the only
supported container runtime.

## Core Concepts

### 1. Build Root Discovery

The tool walks up the directory tree from the current working directory to find a configuration file. This file marks the root of the project and contains configuration parameters.

**Algorithm**:
- Start at current working directory
- Walk up parent directories until `/` is reached
- Look for configuration file in each directory
- Stop at first match and use that directory as the build root

**Config file naming** (TBD for Rust implementation):
- Python version: `.docker_build_root` or `docker_build_root`
- Rust version: Could be `.giftwrap`, `giftwrap.toml`, etc.

### 2. Configuration System

**File Format** (Python version uses simple whitespace-delimited):
```
param_name value1 value2 value3
```

**Note**: Rust implementation may use TOML or other structured format.

**Note**: Rust implementations parameters are TBD

**Required Parameters**:
- `container_image`: The container image to use

**Optional Parameters**:
- `mount_to`: Override mount point inside container (default: same as host path)
- `cd_to`: Directory to cd into inside container
- `extra_shares`: Additional host paths to mount (format: `/host/path` or `/host/path:/container/path`)
- `extra_hosts`: Additional /etc/hosts entries
- `extra_shell`: Shell script to source before running commands
- `env_overrides`: Environment variables to pass through from host
- `prefix_cmd`: Command to run before user command (output visible)
- `prefix_cmd_quiet`: Command to run before user command (output suppressed)
- `version_by_build_context`: Enable container versioning by file content SHA
- `persist_environment`: Path to file for persisting environment between runs
- `prelaunch_hook`: Command to run on host before launching container
- `uuid`: Unique identifier for this configuration (enables UUID-scoped env overrides)
- `extra_args`: Additional arguments to pass to Podman

**Environment Variable Overrides**:

Configuration can be modified via environment variables with special prefixes:
- `GW_USER_OPT_SET_<param>`: Override parameter value
- `GW_USER_OPT_ADD_<param>`: Append to parameter value
- `GW_USER_OPT_DEL_<param>`: Remove parameter
- `GW_USER_OPT_*_UUID_<uuid>_<param>`: Same as above, but only applies when config has matching UUID

**Note**: Python version uses `DR_` prefix; Rust version should use `GW_` (giftwrap).

### 3. Build Context Versioning

**Purpose**: Automatically version container images based on the content of
the build context, ensuring reproducible builds. The build context is a
concept borrowed from docker. In docker all files visible to the dockerfile
are part of the context. In giftwrap a specifically selected subset of the
build root is visible to the container building engine (Podman/buildah).

**Process**:
1. Read an include file, `.giftwrapped` this file follows the pattern of a
   gitignore, and is used to specify all of the files and folders in the
   context. (this was implemented using .dockerignore in the original python
   version)
2. Collect all files matching the rules in `.giftwrapped`
3. Include container definition file (`Containerfile` or `Dockerfile`) and ignore file
4. Expand directories to all contained files
5. Compute SHA1 (or SHA256) hash of all file contents (streamed to handle large files)
6. Cache SHA and file list in a "shafile"
7. Tag container as `image:sha`

**Optimization**: The shafile caches the SHA and list of files. If the shafile is newer than all files in the list, skip recomputation.

### 4. Container Execution Model

**Container Runtime Setup**:
- Auto-remove container after exit
- Interactive mode with stdin
- Mount build root at same path (or custom path via `mount_to`)
- Add TTY allocation if stdin/stdout are TTYs
- Run with appropriate privileges for development (configurable)
- Set hostname to sanitized container name

**Inside Container (Bootstrap code)**:

The tool injects a bootstrap script that runs inside the container to set up the environment.

1. **Change to working directory**: `cd` to the original working directory from host
2. **Create matching user**:
   - Create group with host GID
   - Create user with host UID/GID
   - Set home directory to temporary location
   - Configure sudo access with NOPASSWD
3. **Environment restoration**:
   - If `persist_environment` is set and file exists, load saved environment
   - Apply environment overrides from host
   - Set HOME to temporary home directory
4. **Drop privileges**: Switch to match host user's UID/GID
5. **Terminal setup**: If TTY, install terminfo database for proper terminal emulation
6. **Execute user command**: Replace process with user's shell and command

**Command execution sequence**:
```bash
{
  source extra_shell (if configured)
  prefix_cmd/prefix_cmd_quiet (if configured)
  user_command
  exitcode=$?
  save_environment (if persist_environment configured)
  exit $exitcode
}
```

**Bootstrap Implementation**:
- Python version: Injects Python script via `-c` argument
- Rust version: Could use shell script, or ship a small binary helper, or continue using Python

### 5. Terminal Integration

**Problem**: Terminal capabilities (colors, cursor movement, etc.) require terminfo database matching `$TERM`.

**Solution**:
- Detect if stdin/stdout are TTYs
- Capture `$TERM` from host
- Extract terminfo data via `infocmp $TERM`
- Encode and pass to container
- Inside container, install terminfo to user's home directory via `tic`

### 6. Git Integration

**Problem**: Git can store `.git` directory outside the repository (e.g., git worktrees, submodules).

**Solution**:
- If `share_git_dir` parameter is set
- Run `git rev-parse --git-common-dir` to find actual git directory
- If it's outside the build root, add it as an extra volume mount
- Ensures git commands work correctly inside container

### 7. Command Line Interface

**User Command Execution**:
```
giftwrap [--gw-flags] [-- runtime-flags] command [args...]
```

**Flags** (using `--gw-` prefix for "giftwrap"):
- `--gw-print`: Print container command instead of executing
- `--gw-ctx`: Print the build context SHA and exit
- `--gw-print-image`: Print the image name (with SHA if applicable) and exit
- `--gw-use-ctx=SHA`: Force specific context SHA
- `--gw-img=IMAGE`: Override container image
- `--gw-rebuild`: Rebuild the container image before running
- `--gw-extra-args=ARGS`: Add extra arguments to container runtime
- `--gw-show-config`: Dump parsed configuration and exit
- `--gw-help`: Show help

**Argument Splitting**:
- `--` separator: Everything before goes to container runtime, everything after is user command
- Without `--`: First non-flag argument starts user command

## Key Design Decisions

### Why Create Matching User Inside Container?

1. **File permissions**: Files created in mounted volumes need to have correct ownership
2. **Predictable environment**: User expects to be same user inside/outside container
3. **Security**: Don't run user commands as root

### Why Persist Environment?

Allows persistent development sessions where environment modifications (e.g., `export VAR=value`, `source venv/bin/activate`) survive across container invocations.

### Why Privileged Mode (Optional)?

Enables debugging tools (ptrace, gdb), system-level development, and avoids seccomp restrictions. Trade-off: less isolation, but prioritizes developer convenience. Should be configurable.

### Why Bootstrap Script Inside Container?

Provides flexible, portable way to:
- Create users dynamically
- Handle environment setup
- Drop privileges correctly
- Work across different base images

## Container Runtime

### Podman (Only Supported Runtime)

Giftwrap requires Podman and does not support other container runtimes.

**Why Podman:**
1. **Daemonless**: No background service required, simpler architecture
2. **Fast**: Achieves <50ms container startup for cached images
3. **Docker-compatible CLI**: Eases migration from Python script
4. **Modern**: Better security model, supports rootless containers
5. **Cross-platform**: Works on Linux, macOS, Windows

**Implementation:**
- giftwrap shells out to the `podman` CLI command
- Verifies Podman is available at startup
- Errors with helpful message if Podman is not installed

## Architecture for Rust Implementation

### Module Structure

```
giftwrap/
├── config/           # Configuration file parsing and merging
├── build_context/    # SHA computation and caching
├── podman/           # Podman command generation and execution
├── terminal/         # TTY detection and terminfo handling
├── bootstrap/        # Generate bootstrap code for container
└── main.rs           # CLI argument parsing and orchestration
```

### Key Data Structures

```rust
struct Config {
    container_image: String,
    mount_to: Option<PathBuf>,
    cd_to: Option<PathBuf>,
    extra_shares: Vec<String>,
    extra_hosts: Vec<String>,
    env_overrides: Vec<String>,
    // ... etc
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
fn podman_run(config: &Config, user_ctx: &UserContext) -> Command;
fn podman_build(context: &Path, tag: &str) -> Command;
fn verify_podman_available() -> Result<()>;
```

### Critical Path

1. Verify Podman is available
2. Find config file → Parse → Apply env overrides
3. Compute build context SHA (if enabled)
4. Build Podman command with all mounts and settings
5. Generate bootstrap code
6. Execute Podman (or print if `--gw-print`)

## Compatibility Notes

- Original Python script is Docker-specific
- Rust version requires Podman (no Docker support)
- Config file concepts maintained for migration compatibility
- Environment variable override mechanism preserved (`GW_` prefix instead of `DR_`)
- CLI flags follow similar patterns with `--gw-` prefix
- Migration from Python/Docker version requires installing Podman
