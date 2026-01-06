# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Giftwrap is a container wrapper tool that runs commands inside Podman containers while maintaining proper file permissions, environment variables, and terminal capabilities. It makes containerized development environments feel native.

**Container Runtime**: Podman only (required - will error if not installed)

**Design**: See `DESIGN.md` for complete architecture, design decisions, and implementation details.

**Inspiration**: Python script at `inspiration/docker-runner.py` (uses Docker)

**Status**: Early development - Rust project not yet initialized

## Build Commands

Standard Rust binary:

```bash
# Build
cargo build
cargo build --release

# Test
cargo test
cargo test test_name    # Run specific test

# Development
cargo check             # Fast compilation check
cargo clippy            # Lints
cargo fmt               # Format code

# Run
cargo run -- [giftwrap args]
```

## Module Structure

```
src/
├── main.rs              # CLI parsing, orchestration
├── config/              # Config file parsing, env var overrides
├── build_context/       # SHA computation for context versioning
├── podman/              # Podman command generation and execution
├── terminal/            # TTY detection, terminfo handling
├── bootstrap/           # Bootstrap code generation for containers
└── git.rs               # Git worktree/submodule detection
```

## Key Implementation Notes

**Config System**:
- Config file naming: TBD (see DESIGN.md)
- Config format: TBD - likely TOML (see DESIGN.md)
- Environment variable overrides: `GW_USER_OPT_SET_*`, `GW_USER_OPT_ADD_*`, `GW_USER_OPT_DEL_*`
- UUID-scoped overrides: `GW_USER_OPT_*_UUID_<uuid>_<param>`

**Podman Integration**:
- Shell out to `podman` CLI command
- Verify Podman available at startup
- No runtime abstraction - Podman only
- Target: <50ms container startup for cached images

**Bootstrap Strategy**:
- Implementation method TBD (shell script / Python / Rust binary)
- Must create matching user with host UID/GID inside container
- Must handle privilege dropping, terminfo installation, environment restoration

**Critical Execution Path** (see DESIGN.md for details):
1. Verify Podman available
2. Find config → parse → apply env overrides
3. Compute build context SHA (if enabled)
4. Build Podman command with mounts/privileges/environment
5. Generate bootstrap script
6. Execute (or print if `--gw-print`)

## CLI Flags

All giftwrap flags use `--gw-` prefix:
- `--gw-print`: Print Podman command instead of executing
- `--gw-ctx`: Print build context SHA
- `--gw-print-image`: Print image name with SHA
- `--gw-use-ctx=SHA`: Force specific context SHA
- `--gw-img=IMAGE`: Override container image
- `--gw-rebuild`: Rebuild container before running
- `--gw-extra-args=ARGS`: Pass extra args to Podman
- `--gw-show-config`: Dump parsed configuration
- `--gw-help`: Show help

Argument separator: `--` splits giftwrap/Podman args from user command

## Design Patterns to Follow

See `DESIGN.md` for rationale on:
- Why create matching user inside container (file permissions)
- Why persist environment (stateful dev sessions)
- Why bootstrap script (dynamic user creation, privilege dropping)
- Build context versioning with SHA (reproducible builds)
- Git directory sharing (worktrees/submodules)
