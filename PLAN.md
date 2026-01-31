# giftwrap modernization plan (from conversation)

## Goals
- Preserve user-visible behavior from `inspiration/docker-run.py` while modernizing implementation in Rust.
- Remove the inline Python script injection into the container.
- Use the Podman CLI (not HTTP) for container lifecycle, always via `podman run`.
- Support fully interactive mode by letting Podman own the TTY (`exec podman run -it`).
- Ensure commands launched inside the container are seamlessly usable (e.g., `vim` feels like `podman run -it`).
- Provide a default in-container agent that is baked into the image, is musl-static, and is user-extensible.

## Constraints & source of truth
- Reference behavior: `inspiration/docker-run.py`.
- Config discovery: search upward for `.docker_build_root` or `docker_build_root`.
- Keep CLI flag names and behavior unless intentionally documented changes.

## High-level crate breakdown (two crates)
- Workspace with two crates:
  - `giftwrap` (host CLI, standard libc target)
  - `giftwrap-agent` (in-container, musl-static target)
- `giftwrap` crate modules:
  - `config`
    - Config discovery + parsing + validation.
    - Apply `GW_USER_OPT_{SET,ADD,DEL}_*` environment overrides (replacement for legacy `DR_USER_OPT_{SET,ADD,DEL}_*`), with UUID scoping.
    - Output: `Config`.
  - `context`
    - Git-style file selection + `.gwinclude` (top-level and nested) semantics (replacement for `.dockerignore` negated patterns) + context SHA + SHA file management.
    - Output: `ContextSha` + file list metadata.
  - `cli`
    - Parse `--gw-*` flags (replacement for legacy `--dr-*`) and split docker args vs user command via `--` delimiter.
    - Output: `CliOptions`, `UserCommand`.
  - `compose`
    - Pure builder: map `Config + CliOptions + HostInfo` to a `RunSpec`/`ContainerSpec`.
    - Handles mounts, extra shares, extra hosts, git-dir sharing, env overrides, workdir/mount mapping, terminfo decisions.
  - `internal` (agent API definitions)
    - Shared `RunSpec`/`InternalSpec` types and protocol versioning for agent (kept in sync with agent).
    - Serialization format and compatibility rules.
  - `persist`
    - Persisted environment read/write format and compatibility.
  - `exec`
    - Side effects: prelaunch hook, rebuild/build, exec/print, host probes (isatty, ARG_MAX, infocmp, git).
- `giftwrap-agent` crate modules:
  - `internal` (agent API definitions mirrored from `giftwrap`).
  - Agent runtime modules for user setup, env handling, and exec.

## Remove injected Python; replace with baked-in agent
### Decision
- Do not inject Python into the container.
- Use a default `giftwrap-agent` baked into the base image and set as entrypoint.

### Agent requirements
- `giftwrap-agent` compiled as **musl static** Rust binary.
- Handles in-container setup:
  - Optional user identity mapping or `keep-id` style behavior depending on runtime plan.
  - HOME/workdir setup.
  - Env overrides + persisted env restore/save.
  - TERM/terminfo handling.
  - Exec the user command, propagate exit code.

### Extensibility
- Provide a defined internal API (`internal` module) for spec and hooks.
- Allow users to extend behavior by:
  - Replacing the agent with a custom build using the same API, or
  - Adding plugins discovered by the default agent.

## Podman integration: CLI-based
### Decision
- Use Podman CLI exclusively.
- No Podman REST API or libpod bindings.

### Podman CLI wrapper
- `podman_cli` module provides:
  - `build_image`, `inspect_image`, `run`.
- Compose Podman CLI args, and `exec` Podman so it directly owns stdin/stdout/stderr.
- Always include `--rm` with `podman run`.
- Clear error mapping to user-visible messages before `exec`.

### Interactive mode (TTY ownership via exec)
- Always `exec` Podman (`podman run -it`) so it directly owns the TTY/FDS.
- Let Podman handle raw mode, SIGWINCH, and resize propagation.
- Preserve control sequences and signals (Ctrl+C, Ctrl+Z, etc.) via Podman’s native TTY handling.
- If not a TTY, fall back to non-interactive `podman` invocation.

## Behavior parity targets
- Preserve all `--gw-*` flags (replacement for legacy `--dr-*`):
  - print, ctx, print-image, use-ctx, img, rebuild, show-config, extra-args, help.
- Maintain build-context SHA tagging behavior with git-style file selection and `.gwinclude` semantics (replacement for `.dockerignore` negated rules).
- Preserve shared mount semantics and optional git-dir sharing.
- Preserve persisted environment behavior (new implementation but same semantics).

## Current status
- Workspace split into two crates: `giftwrap` (host CLI) and `giftwrap-agent` (agent).
- Shared internal data models defined and mirrored in the agent crate.
- Config discovery/parsing + GW_USER_OPT_* environment overrides implemented.
- CLI flag parsing implemented (including `--` split handling).

## Next steps (implementation sequence)
- [x] Define shared data models: `Config`, `CliOptions`, `RunSpec`, `InternalSpec`, `ContainerSpec`.
- [x] Implement `config` and `cli` modules to match legacy behavior.
- [ ] Implement `context` module to match git-style file selection + `.gwinclude` semantics + SHA logic.
- [ ] Implement agent runtime pieces (user setup, env handling, exec) and musl-static build config.
- [ ] Implement `podman_cli` module and wire into `exec`.
- [ ] Replace old docker CLI invocation with Podman CLI `run`.
- [ ] Add PTY bridging and resize handling for interactive mode.
- [ ] Validate parity with the Python script on key flows.
