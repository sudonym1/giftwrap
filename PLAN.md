# giftwrap modernization plan (from conversation)

## Goals
- Preserve user-visible behavior from the legacy script in `inspiration/` while modernizing implementation in Rust.
- Remove the inline Python script injection into the container.
- Use the container runtime CLI (not HTTP) for container lifecycle, always via `podman run`.
- Support fully interactive mode by letting the runtime own the TTY (`exec podman run -it`).
- Ensure commands launched inside the container are seamlessly usable (e.g., `vim` feels like `podman run -it`).
- Provide a single musl-compiled `giftwrap` binary that serves as both host CLI and in-container agent (`giftwrap agent ...`), bind-mounted into the container.

## Constraints & source of truth
- Reference behavior: legacy script in `inspiration/`.
- Config discovery: search upward for `.giftwrap` or `giftwrap`.
- Keep CLI flag names and behavior unless intentionally documented changes.

## High-level crate breakdown (single crate)
- Single crate at repo root: `giftwrap` (host CLI + in-container agent subcommand).
- Modules:
  - `config`
    - Config discovery + parsing + validation.
    - Apply `GW_USER_OPT_{SET,ADD,DEL}_*` environment overrides (replacement for legacy `DR_USER_OPT_{SET,ADD,DEL}_*`), with UUID scoping.
    - Output: `Config`.
  - `context`
    - Git-style file selection + `.gwinclude` (top-level and nested) semantics + context SHA + SHA file management.
    - Output: `ContextSha` + file list metadata.
  - `cli`
    - Parse `--gw-*` flags (replacement for legacy `--dr-*`) and split runtime args vs user command via `--` delimiter.
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
  - `agent`
    - In-container agent runtime (user setup, env handling, terminfo handling, exec).

## Remove injected Python; replace with bind-mounted subcommand
### Decision
- Do not inject Python into the container.
- Bind-mount the host `giftwrap` binary into the container and set entrypoint to `giftwrap agent`.

### Agent requirements
- `giftwrap` compiled as **musl static** Rust binary.
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

## Container runtime integration: CLI-based
### Decision
- Use the runtime CLI exclusively.
- No runtime REST API or libpod bindings.

### Runtime CLI wrapper
- Runtime CLI wrapper provides:
  - `build_image`, `inspect_image`, `run`.
- Compose runtime CLI args, and `exec` the runtime so it directly owns stdin/stdout/stderr.
- Always include `--rm` with `podman run`.
- Clear error mapping to user-visible messages before `exec`.

### Interactive mode (TTY ownership via exec)
- Always `exec` the runtime (`podman run -it`) so it directly owns the TTY/FDS.
- Let the runtime handle raw mode, SIGWINCH, and resize propagation.
- Preserve control sequences and signals (Ctrl+C, Ctrl+Z, etc.) via the runtime’s native TTY handling.
- If not a TTY, fall back to non-interactive runtime invocation.

## Behavior parity targets
- Preserve all `--gw-*` flags (replacement for legacy `--dr-*`):
  - print, ctx, print-image, use-ctx, img, rebuild, show-config, extra-args, help.
- Maintain build-context SHA tagging behavior with `.gwinclude` selection rules.
- Preserve shared mount semantics and optional git-dir sharing.
- Preserve persisted environment behavior (new implementation but same semantics).

## Current status
- Single root `giftwrap` crate with host + agent subcommand.
- Shared internal data models defined once and used by both host + agent.
- Config discovery/parsing + GW_USER_OPT_* environment overrides implemented.
- CLI flag parsing implemented (including `--` split handling).
- Context module implemented (gwinclude-only selection + SHA + sha-file reuse).
- Config discovery now uses `.giftwrap` / `giftwrap`, and config keys are prefixed with `gw` where applicable.
- Runtime CLI wrapper implemented with build/inspect/run and exec wiring.
- CLI now composes a minimal runtime run from config/flags and executes it.
- Host now bind-mounts the unified `giftwrap` binary (prefers musl builds) and uses it as the entrypoint.
- Agent runtime implemented (user setup, env handling, terminfo, exec) with Alpine-friendly fallbacks.

## Next steps (implementation sequence)
- [x] Define shared data models: `Config`, `CliOptions`, `RunSpec`, `InternalSpec`, `ContainerSpec`.
- [x] Implement `config` and `cli` modules to match legacy behavior.
- [x] Implement `context` module to match git-style file selection + `.gwinclude` semantics + SHA logic.
- [x] Implement agent runtime pieces (user setup, env handling, exec).
- [x] Collapse workspace to a single top-level `giftwrap` crate.
- [x] Move current `giftwrap` crate contents to repo root crate.
- [x] Fold agent functionality into `giftwrap agent` subcommand.
- [x] Update runtime to bind-mount the unified `giftwrap` binary and set entrypoint to `giftwrap agent`.
- [ ] Add musl-static build config/docs for unified `giftwrap` binary (target config + release guidance).
- [x] Implement runtime CLI module and wire into `exec`.
- [x] Replace old runtime CLI invocation with `podman run`.
- [ ] Validate parity with the Python script on key flows.
