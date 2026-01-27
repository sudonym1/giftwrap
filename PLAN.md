# giftwrap modernization plan (from conversation)

## Goals
- Preserve user-visible behavior from `inspiration/docker-run.py` while modernizing implementation in Rust.
- Remove the inline Python script injection into the container.
- Make Podman integration HTTP-only (no CLI arg building, no library bindings).
- Support interactive mode by bridging the user’s terminal/PTY to the Podman attach stream.
- Provide a default in-container agent that is baked into the image, is musl-static, and is user-extensible.

## Constraints & source of truth
- Reference behavior: `inspiration/docker-run.py`.
- Config discovery: search upward for `.docker_build_root` or `docker_build_root`.
- Keep CLI flag names and behavior unless intentionally documented changes.

## High-level crate breakdown (orthogonal responsibilities)
- `giftwrap-config`
  - Config discovery + parsing + validation.
  - Apply `DR_USER_OPT_{SET,ADD,DEL}_*` environment overrides, with UUID scoping.
  - Output: `Config`.
- `giftwrap-context`
  - `.dockerignore` negated patterns handling + context SHA + SHA file management.
  - Output: `ContextSha` + file list metadata.
- `giftwrap-cli`
  - Parse `--dr-*` flags and split docker args vs user command via `--` delimiter.
  - Output: `CliOptions`, `UserCommand`.
- `giftwrap-compose`
  - Pure builder: map `Config + CliOptions + HostInfo` to a `RunSpec`/`ContainerSpec`.
  - Handles mounts, extra shares, extra hosts, git-dir sharing, env overrides, workdir/mount mapping, terminfo decisions.
- `giftwrap-internal` (agent API definitions)
  - Shared `RunSpec`/`InternalSpec` types and protocol versioning for agent.
  - Serialization format and compatibility rules.
- `giftwrap-persist`
  - Persisted environment read/write format and compatibility.
- `giftwrap-exec`
  - Side effects: prelaunch hook, rebuild/build, exec/print, host probes (isatty, ARG_MAX, infocmp, git).
- `giftwrap` (bin)
  - Orchestrates flow only; no business logic.

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
- Provide a defined internal API (`giftwrap-agent-api` crate) for spec and hooks.
- Allow users to extend behavior by:
  - Replacing the agent with a custom build using the same API, or
  - Adding plugins discovered by the default agent.

## Podman integration: HTTP-only
### Decision
- Use Podman REST API exclusively.
- No Podman CLI invocation and no libpod bindings.

### Podman HTTP client
- `podman-http` crate provides:
  - `build_image`, `inspect_image`, `create_container`, `start_container`,
    `attach`, `wait`, `remove`, `logs`.
- Connection over Unix domain socket (rootless or rootful).
- Optional API version negotiation at startup.
- Clear error mapping to user-visible messages.

### Interactive mode (PTY bridging)
- HTTP attach hijacks the connection; use it as the data stream.
- Put user terminal into raw mode.
- Bridge bytes:
  - stdin -> attach stream
  - attach stream -> stdout/stderr
- Handle `SIGWINCH` and call container resize endpoint to keep TTY size in sync.
- If not a TTY, fall back to non-interactive attach.

## Behavior parity targets
- Preserve all `--dr-*` flags:
  - print, ctx, print-image, use-ctx, img, rebuild, show-config, extra-args, help.
- Maintain build-context SHA tagging behavior with `.dockerignore` negated rules.
- Preserve shared mount semantics and optional git-dir sharing.
- Preserve persisted environment behavior (new implementation but same semantics).

## Next steps (implementation sequence)
1) Define shared data models: `Config`, `CliOptions`, `RunSpec`, `InternalSpec`, `ContainerSpec`.
2) Implement `giftwrap-config` and `giftwrap-cli` to match legacy behavior.
3) Implement `giftwrap-context` to match `.dockerignore` + SHA logic.
4) Implement `giftwrap-agent-api` and skeleton `giftwrap-agent` (musl static build).
5) Implement `podman-http` client and wire into `giftwrap-exec`.
6) Replace old docker CLI invocation with HTTP create/start/attach/wait.
7) Add PTY bridging and resize handling for interactive mode.
8) Validate parity with the Python script on key flows.

