# giftwrap

## Project summary
`giftwrap` is a Rust rewrite and modernization of the legacy script in `inspiration/`. It is a CLI wrapper around a container runtime (defaulting to rootless containers) that discovers a build root, reads a config file, optionally rebuilds/tag-bumps an image based on build context, and launches a container with user/UID/GID mapping, shared volumes, and environment handling.

## Source of truth for behavior
- The legacy script in `inspiration/` is the reference implementation. Preserve its user-visible behavior unless the port explicitly modernizes or documents a change.
- Config discovery searches upward from the current working directory for `.giftwrap` or `giftwrap` and uses that directory as the build root.

## Key behaviors to preserve (high level)
- Parse the config file into parameters; allow environment variables to add/set/delete options.
- Optional context SHA tagging based on `.gwinclude` selection rules.
- `--gw-*` flags for printing, rebuilding, overriding image/tag, extra runtime args, config dump, etc.
- Compose the runtime invocation (rootless by default) with mounts of the build root, extra shares, and optional git-dir sharing.
- Optional prelaunch hook, extra shell sourcing, and prefix commands.
- Persisted environment feature that round-trips environment variables between runs.
- Inside-container setup: create user matching host UID/GID, apply environment overrides, set HOME, handle TERM/terminfo, then exec the requested command.

## Repository layout
- `src/` Rust source for the new CLI.
- `inspiration/` legacy Python implementation used for parity checks.

## Build and run
- Build: `cargo build`
- Run: `cargo run -- <args>`
- Tests: `cargo test` (unit tests only).

## Agent restrictions (read carefully)
- NEVER run the integration test suite or any script that invokes `podman`.
- The agent sandbox cannot run containers; integration runs will hang or fail.
- Integration runs are manual-only on a developer machine and should write
  artifacts to `artifacts/integration/` for later inspection.

## Conventions
- Keep the Rust CLI name `giftwrap` and stay compatible with existing flag names unless the change is intentional and documented.
- Prefer explicit error messages and exit codes that match the Python script where practical.
- Use Rust 2024 edition defaults and keep dependencies minimal unless a crate clearly reduces risk or complexity.
