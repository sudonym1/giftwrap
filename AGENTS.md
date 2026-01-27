# giftwrap

## Project summary
`giftwrap` is a Rust rewrite and modernization of `inspiration/docker-run.py`. It is a CLI wrapper around Podman (defaulting to rootless containers) that discovers a build root, reads a config file, optionally rebuilds/tag-bumps an image based on build context, and launches a container with user/UID/GID mapping, shared volumes, and environment handling.

## Source of truth for behavior
- `inspiration/docker-run.py` is the reference implementation. Preserve its user-visible behavior unless the port explicitly modernizes or documents a change.
- Config discovery searches upward from the current working directory for `.docker_build_root` or `docker_build_root` and uses that directory as the build root.

## Key behaviors to preserve (high level)
- Parse the config file into parameters; allow environment variables to add/set/delete options.
- Optional context SHA tagging based on `.dockerignore` with negated patterns and `Dockerfile`.
- `--dr-*` flags for printing, rebuilding, overriding image/tag, extra docker args, config dump, etc.
- Compose the Podman invocation (rootless by default) with mounts of the build root, extra shares, and optional git-dir sharing.
- Optional prelaunch hook, extra shell sourcing, and prefix commands.
- Persisted environment feature that round-trips environment variables between runs.
- Inside-container setup: create user matching host UID/GID, apply environment overrides, set HOME, handle TERM/terminfo, then exec the requested command.

## Repository layout
- `src/` Rust source for the new CLI.
- `inspiration/` legacy Python implementation used for parity checks.

## Build and run
- Build: `cargo build`
- Run: `cargo run -- <args>`
- Tests: `cargo test` (no tests yet).

## Conventions
- Keep the Rust CLI name `giftwrap` and stay compatible with existing flag names unless the change is intentional and documented.
- Prefer explicit error messages and exit codes that match the Python script where practical.
- Use Rust 2024 edition defaults and keep dependencies minimal unless a crate clearly reduces risk or complexity.
