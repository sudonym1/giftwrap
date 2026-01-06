# Phased Implementation Plan for Giftwrap

## Overview

This plan builds giftwrap incrementally in **6 phases**, where each phase adds a cohesive layer of functionality that can be tested independently. The approach follows the "walking skeleton" pattern - establishing end-to-end functionality early, then progressively adding features.

**Current State:**
- Basic Cargo project initialized (Hello World)
- Comprehensive DESIGN.md with architecture
- Reference Python implementation (docker-runner.py)
- Clear module structure defined

**Target:**
- Complete Podman wrapper tool in Rust
- ~50ms container startup for cached images
- Full feature parity with reference implementation

---

## Phase 1: Project Skeleton & Basic CLI

**Goal:** Establish compilable project structure with argument parsing and help system

**Functionality:**
- Basic CLI argument parsing (using `clap` crate)
- All `--gw-*` flags defined (even if not yet implemented)
- Argument separator `--` handling
- Help text and version information
- Project module structure created (empty modules)

**Files to Create/Modify:**
- `Cargo.toml` - Add dependencies (clap, anyhow, thiserror)
- `src/main.rs` - CLI parsing and main orchestration
- `src/lib.rs` - Library root with module declarations
- `src/config/mod.rs` - Empty config module
- `src/podman/mod.rs` - Empty podman module
- `src/build_context/mod.rs` - Empty build context module
- `src/terminal/mod.rs` - Empty terminal module
- `src/bootstrap/mod.rs` - Empty bootstrap module
- `src/git.rs` - Empty git module

**Key Data Structures:**
```rust
struct CliArgs {
    gw_print: bool,
    gw_ctx: bool,
    gw_print_image: bool,
    gw_use_ctx: Option<String>,
    gw_img: Option<String>,
    gw_rebuild: bool,
    gw_extra_args: Vec<String>,
    gw_show_config: bool,
    runtime_args: Vec<String>,
    user_command: Vec<String>,
}
```

**Tests:**
- Parse `--gw-help` and show help
- Parse `--gw-print-image` flag
- Split arguments around `--` separator
- Parse `--gw-extra-args="arg1 arg2"`
- Integration test: `cargo run -- --gw-help`

**Success Criteria:**
- `cargo build` compiles successfully
- `cargo run -- --gw-help` shows help text
- All CLI flags parse correctly
- Arguments split properly around `--`

---

## Phase 2: Config Discovery & Parsing

**Goal:** Implement configuration file discovery, parsing, and environment variable overrides

**Functionality:**
- Walk up directory tree to find config file (`.giftwrap`, `giftwrap.toml`, or `.giftwrap.toml`)
- Parse config file (whitespace-delimited format initially)
- Apply environment variable overrides (`GW_USER_OPT_SET_*`, `GW_USER_OPT_ADD_*`, `GW_USER_OPT_DEL_*`)
- Handle UUID-scoped overrides
- Implement `--gw-show-config` to dump parsed configuration

**Files to Create/Modify:**
- `src/config/mod.rs` - Config discovery and parsing (CRITICAL)
- `src/config/parser.rs` - Config file format parser
- `src/config/env_override.rs` - Environment variable override logic
- `src/main.rs` - Wire config loading into main flow

**Key Data Structures:**
```rust
pub struct Config {
    pub container_image: String,
    pub mount_to: Option<PathBuf>,
    pub cd_to: Option<PathBuf>,
    pub extra_shares: Vec<String>,
    pub extra_hosts: Vec<String>,
    pub env_overrides: Vec<String>,
    pub prefix_cmd: Option<Vec<String>>,
    pub prefix_cmd_quiet: Option<Vec<String>>,
    pub version_by_build_context: Option<String>,
    pub persist_environment: Option<PathBuf>,
    pub prelaunch_hook: Option<Vec<String>>,
    pub uuid: Option<String>,
    pub extra_args: Vec<String>,
    pub share_git_dir: bool,
    pub extra_shell: Option<PathBuf>,
}

pub fn find_config() -> Result<(PathBuf, PathBuf)>; // (config_file, build_root)
pub fn parse_config(path: &Path) -> Result<Config>;
pub fn apply_env_overrides(config: &mut Config) -> Result<()>;
```

**Tests:**
- Parse valid config file
- Handle missing required parameter (`container_image`)
- Apply `GW_USER_OPT_SET_*` override
- Apply `GW_USER_OPT_ADD_*` override
- Apply `GW_USER_OPT_DEL_*` override
- UUID-scoped overrides only apply when UUID matches
- Integration: Create test config, verify discovery
- Integration: `--gw-show-config` displays configuration

**Success Criteria:**
- Config file discovery works from any subdirectory
- Config parsing handles all parameters
- Environment overrides apply correctly
- `--gw-show-config` displays parsed configuration
- Clear error messages when config is missing/invalid

---

## Phase 3: Podman Verification & Basic Execution

**Goal:** Verify Podman availability and execute simple container commands

**Functionality:**
- Check if Podman is installed and available
- Build basic Podman `run` command from config
- Handle volume mounts (build root, extra shares)
- Handle TTY allocation (`-t` flag)
- Execute Podman command (or print if `--gw-print`)
- Run simple commands without bootstrap initially

**Files to Create/Modify:**
- `src/podman/mod.rs` - Podman interaction (CRITICAL)
- `src/podman/verify.rs` - Verify Podman installation
- `src/podman/command.rs` - Build Podman command
- `src/main.rs` - Wire Podman execution into main flow

**Key Data Structures:**
```rust
pub struct UserContext {
    pub username: String,
    pub uid: u32,
    pub gid: u32,
    pub cwd: PathBuf,
}

pub fn verify_podman_available() -> Result<()>;
pub fn build_run_command(config: &Config, user_ctx: &UserContext) -> Command;
pub fn execute_podman(cmd: Command) -> Result<i32>;
pub fn get_user_context() -> Result<UserContext>;
```

**Podman Command Construction:**
- Base: `podman run -i --rm`
- Add `-t` if stdin/stdout are TTYs
- Add volume mounts: `-v build_root:mount_to`
- Add extra shares
- Add hostname: `-h <sanitized-container-name>`
- Add `--privileged=true` (make configurable later)

**Tests:**
- `verify_podman_available()` detects Podman
- Build basic Podman command from minimal config
- Add volume mounts correctly
- TTY detection
- Hostname sanitization (replace invalid chars)
- Integration: Run simple command in container (`echo hello`)
- Integration: `--gw-print` shows Podman command

**Success Criteria:**
- Podman availability check works
- Can execute simple commands in containers
- `--gw-print` displays correct Podman command
- Volume mounts work (verify file access)
- TTY allocation works correctly

---

## Phase 4: Bootstrap Script & User Creation

**Goal:** Generate and inject bootstrap script to create matching user and set up container environment

**Functionality:**
- Generate bootstrap script (shell script recommended for simplicity)
- Create matching user with host UID/GID inside container
- Set up temporary home directory
- Configure sudo access (NOPASSWD)
- Drop privileges to match host user
- Execute user command with proper environment
- Handle environment restoration (if `persist_environment` configured)

**Files to Create/Modify:**
- `src/bootstrap/mod.rs` - Bootstrap generation (CRITICAL)
- `src/bootstrap/script.rs` - Script template and generation
- `src/podman/command.rs` - Inject bootstrap into Podman command
- `src/main.rs` - Wire bootstrap into execution flow

**Bootstrap Implementation:** Shell script (Option A) - Simple, no dependencies, portable

**Key Functions:**
```rust
pub fn generate_bootstrap_script(
    config: &Config,
    user_ctx: &UserContext,
    user_command: &[String],
    env_overrides: &HashMap<String, String>,
) -> Result<String>;

pub struct BootstrapConfig {
    pub cd_to: PathBuf,
    pub user: String,
    pub uid: u32,
    pub gid: u32,
    pub home_dir: PathBuf,
    pub env_file: Option<PathBuf>,
    pub restore_env: bool,
    pub env_overrides: HashMap<String, String>,
    pub user_command: Vec<String>,
}
```

**Bootstrap Flow:**
1. Change to working directory
2. Create group and user with host UID/GID
3. Configure sudo access
4. Restore environment (if configured)
5. Apply environment overrides
6. Drop privileges and execute user command

**Tests:**
- Generate bootstrap script with minimal config
- Generate bootstrap with environment restoration
- Generate bootstrap with environment overrides
- Escape special characters in user command
- Integration: Run command in container with bootstrap
- Integration: Verify files created have correct UID/GID
- Integration: Verify environment variables are set

**Success Criteria:**
- Bootstrap script creates user with correct UID/GID
- Files created in mounted volumes have correct ownership
- User command executes with dropped privileges
- Environment variables are set correctly
- `persist_environment` saves and restores environment

---

## Phase 5: Build Context SHA & Terminal Integration

### 5A: Build Context SHA

**Functionality:**
- Parse `.giftwrapped` file (gitignore-style patterns)
- Collect files matching patterns
- Compute SHA256 hash of file contents
- Cache SHA and file list in shafile
- Tag container image with SHA
- Implement `--gw-ctx` and `--gw-print-image` flags
- Handle `--gw-use-ctx=SHA` override
- Implement `--gw-rebuild` to rebuild container

**Files to Create/Modify:**
- `src/build_context/mod.rs` - Build context logic (CRITICAL)
- `src/build_context/gitignore.rs` - Parse gitignore-style patterns
- `src/build_context/sha.rs` - SHA computation and caching
- `src/podman/build.rs` - Podman build command
- `src/main.rs` - Wire build context into flow

**Key Functions:**
```rust
pub struct BuildContext {
    pub files: Vec<PathBuf>,
    pub sha: String,
}

pub fn parse_giftwrapped(path: &Path) -> Result<Vec<String>>;
pub fn collect_files(root: &Path, patterns: &[String]) -> Result<Vec<PathBuf>>;
pub fn compute_sha(files: &[PathBuf]) -> Result<String>;
pub fn load_or_compute_sha(shafile: &Path, root: &Path) -> Result<BuildContext>;
pub fn is_shafile_dirty(shafile: &Path, files: &[PathBuf]) -> bool;
```

**Tests:**
- Parse `.giftwrapped` patterns
- Collect files matching patterns
- Compute SHA for file list
- Detect dirty shafile (file modified)
- Use cached SHA when files unchanged
- Integration: `--gw-ctx` prints SHA
- Integration: `--gw-print-image` shows image:sha
- Integration: `--gw-use-ctx=CUSTOM` overrides SHA

### 5B: Terminal Integration

**Functionality:**
- Detect if stdin/stdout are TTYs
- Capture `$TERM` environment variable
- Extract terminfo data via `infocmp`
- Encode and pass to container
- Install terminfo inside container via bootstrap script

**Files to Create/Modify:**
- `src/terminal/mod.rs` - Terminal handling (CRITICAL)
- `src/terminal/tty.rs` - TTY detection
- `src/terminal/terminfo.rs` - Terminfo extraction
- `src/bootstrap/script.rs` - Add terminfo installation to bootstrap

**Key Functions:**
```rust
pub struct TerminalInfo {
    pub is_tty: bool,
    pub term: Option<String>,
    pub terminfo: Option<String>, // Base64-encoded
}

pub fn is_tty() -> bool;
pub fn get_term_env() -> Option<String>;
pub fn extract_terminfo(term: &str) -> Result<Vec<u8>>;
pub fn encode_terminfo(data: &[u8]) -> String;
pub fn get_terminal_info() -> Result<TerminalInfo>;
```

**Tests:**
- TTY detection
- Extract terminfo for common TERM values
- Base64 encoding
- Integration: Run command with TTY and verify terminal works
- Integration: Colors work inside container

**Success Criteria:**
- Build context SHA computation works
- SHA caching optimization works
- `--gw-ctx` and `--gw-print-image` work correctly
- Container image tagged with SHA
- `--gw-rebuild` rebuilds container
- TTY detection works
- Terminfo installed correctly in container
- Terminal colors and capabilities work

---

## Phase 6: Git Integration & Polish

**Goal:** Add Git directory sharing, prelaunch hooks, and final polish

**Functionality:**
- Detect Git worktrees/submodules
- Share git directory if outside build root
- Execute prelaunch hooks
- Handle `prefix_cmd` and `prefix_cmd_quiet`
- Environment persistence (`persist_environment`)
- Extra shell sourcing (`extra_shell`)
- Error handling improvements
- Performance optimization (target <50ms container startup)

**Files to Create/Modify:**
- `src/git.rs` - Git integration
- `src/podman/command.rs` - Add git dir mount
- `src/bootstrap/script.rs` - Add prefix commands and environment persistence
- `src/main.rs` - Add prelaunch hook execution

**Key Functions:**
```rust
pub fn should_share_git_dir(config: &Config) -> bool;
pub fn find_git_common_dir() -> Result<Option<PathBuf>>;
pub fn is_outside_build_root(git_dir: &Path, build_root: &Path) -> bool;
```

**Tests:**
- Detect git common dir
- Determine if git dir is outside build root
- Integration: Git commands work in container (with worktree setup)
- Integration: Prelaunch hook executes on host
- Integration: `prefix_cmd` executes before user command
- Integration: `prefix_cmd_quiet` suppresses output
- Integration: Environment persists across invocations
- Integration: `extra_shell` sources correctly
- Performance: Measure container startup time (target <50ms)

**Success Criteria:**
- Git integration works (worktrees, submodules)
- Prelaunch hooks execute correctly
- Prefix commands work (both normal and quiet)
- Environment persistence works
- Extra shell sourcing works
- All CLI flags functional
- Helpful error messages
- Performance target achieved

---

## Dependencies & Crates

```toml
[dependencies]
anyhow = "1.0"           # Error handling
thiserror = "1.0"        # Custom error types
clap = { version = "4.5", features = ["derive"] }
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"             # TOML parsing (if using TOML config)
sha2 = "0.10"            # SHA256 hashing
base64 = "0.22"          # Base64 encoding
nix = { version = "0.29", features = ["user"] }
walkdir = "2.5"          # Directory walking
ignore = "0.4"           # Gitignore-style pattern matching

[dev-dependencies]
tempfile = "3.12"
assert_cmd = "2.0"
predicates = "3.1"
```

---

## Critical Implementation Files

**Top 5 Most Critical (in order):**
1. `src/main.rs` - Orchestration and CLI entry point
2. `src/config/mod.rs` - Config discovery and parsing
3. `src/podman/mod.rs` - Podman command generation and execution
4. `src/bootstrap/mod.rs` - Bootstrap script generation
5. `src/build_context/mod.rs` - Build context SHA computation

---

## Testing Strategy

- **Unit Tests:** Each module with `#[cfg(test)]`
- **Integration Tests:** In `tests/` directory
- **Test Fixtures:** Sample configs, .giftwrapped files
- **CI/CD:** Mark Podman-requiring tests appropriately

---

## Migration from Python Script

For users migrating from `docker-runner.py`:
1. Rename `.docker_build_root` → `.giftwrap`
2. Change `DR_` prefix → `GW_` in environment variables
3. Change `--dr-` prefix → `--gw-` in CLI flags
4. Install Podman
5. Rename `.dockerignore` → `.giftwrapped`
