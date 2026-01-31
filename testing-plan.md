## Giftwrap test plan (no container runtime)

### Context
This repo is a Rust rewrite of the legacy script in `inspiration/`. Tests must
preserve user-visible behavior while avoiding any runtime invocation (no
`podman`, no containers). THE REASON THESE TESTS MUST NOT INVOKE THE RUNTIME
IS THAT THE AGENT SANDBOX WILL PREVENT THEM FROM RUNNING. The goal is to add
coverage one suite at a time with clear boundaries and fixtures.

### Goals
- Cover core behavior without invoking `podman`.
- Keep suites small, deterministic, and fast.
- Mirror legacy behavior where applicable, with explicit expectations.

### Non-goals
- No integration tests that launch containers.
- No network access or external runtime dependencies.

### Suite order (one at a time)
1. CLI parsing
   - `--gw-*` flags (print, ctx, print-image, show-config, help, rebuild)
   - `--gw-extra-args` splitting and error handling
   - `--` delimiter behavior (runtime args vs user command)
   - Terminal actions stop further parsing

2. Runtime arg composition (pure)
   - `ContainerSpec` -> `podman_cli::build_run_args`
   - env, mounts, entrypoint, hostname, user, workdir, flags ordering
   - entrypoint validation (single element or error)

3. Hostname and path helpers
   - `mkhostname` sanitization and length cap
   - share expansion, path resolution, git-dir sharing behavior

4. Config discovery
   - Search upward for `.giftwrap` or `giftwrap`
   - Build root selection from discovery
   - Error paths when missing

5. Config parsing + env overrides
   - Key/value parsing
   - Add/set/delete semantics via env vars
   - Precedence (config vs env vs CLI)

6. Context hashing
   - `.gwinclude` inclusion/exclusion rules
   - Deterministic ordering and hash stability
   - Error conditions (missing or malformed includes)

7. Image/tag selection
   - Default image behavior
   - `--gw-img`, `--gw-use-ctx` overrides
   - Rebuild flag affects flow (but no runtime calls)

8. Internal spec generation
   - JSON shape and fields
   - UID/GID/home, env overrides, terminfo, prefix/extra shell
   - Persisted environment paths

### Status
- [x] Suite 1: CLI parsing (tests added in `src/cli.rs`, `cargo test` passes)
- [x] Suite 2: Runtime arg composition (tests added in `src/podman_cli.rs`, `cargo test` passes)
- [ ] Suite 3: Hostname and path helpers
- [x] Suite 4: Config discovery
- [ ] Suite 5: Config parsing + env overrides
- [ ] Suite 6: Context hashing
- [ ] Suite 7: Image/tag selection
- [ ] Suite 8: Internal spec generation

### Approach per suite
- Add unit tests close to the module under test (`src/*.rs`) where possible.
- Prefer small, focused tests with explicit fixtures.
- Avoid touching `podman_cli::exec_run` or anything that executes processes.
- Use temporary directories and in-memory fixtures; do not rely on global state.

### Fixtures
- Use `tempfile` for filesystem fixtures (if needed).
- Keep fixture trees minimal, with explicit file contents.
- Preserve legacy parity by referencing `inspiration/` for expected behavior where needed.

### Incremental workflow
For each suite:
- Add tests and any minimal helper functions.
- Run `cargo test` as part of your loop in adding each suite without asking.
- Summarize new coverage and any behavior gaps discovered vs legacy script.
- Do not fix and suspected bugs in `giftwrap` without first prompting for
  direction.
