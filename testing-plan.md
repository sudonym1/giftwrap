## Giftwrap test plan

### Context
This repo is a Rust rewrite of the legacy script in `inspiration/`. Unit tests
must preserve user-visible behavior while avoiding any runtime invocation (no
`podman`, no containers). THE REASON THESE TESTS MUST NOT INVOKE THE RUNTIME
IS THAT THE AGENT SANDBOX WILL PREVENT THEM FROM RUNNING. A separate manual
integration suite uses `podman` for end-to-end coverage and is never run by
the agent. The goal is to add coverage one suite at a time with clear
boundaries and fixtures.

### Goals
- Cover core behavior without invoking `podman` (unit suite).
- Keep suites small, deterministic, and fast.
- Mirror legacy behavior where applicable, with explicit expectations.
- Provide a manual integration suite that exercises `podman` and captures
  artifacts in-repo for triage.

### Non-goals
- No integration tests run by the agent or inside the sandbox.
- No network access or external runtime dependencies for the unit suite.

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
9. Integration (manual, `podman` required)
   - End-to-end flows that run containers with simple commands
   - Exercise shares, env overrides, context tagging, and hooks
   - Capture artifacts in-repo for inspection

### Status
- [x] Suite 1: CLI parsing (tests added in `src/cli.rs`, `cargo test` passes)
- [x] Suite 2: Runtime arg composition (tests added in `src/podman_cli.rs`, `cargo test` passes)
- [x] Suite 3: Hostname and path helpers
- [x] Suite 4: Config discovery
- [ ] Suite 5: Config parsing + env overrides
- [x] Suite 6: Context hashing
- [ ] Suite 7: Image/tag selection
- [ ] Suite 8: Internal spec generation
- [ ] Suite 9: Integration (manual, `podman` required; never run by agent)

### Approach per suite
- Add unit tests close to the module under test (`src/*.rs`) where possible.
- Prefer small, focused tests with explicit fixtures.
- Avoid touching `podman_cli::exec_run` or anything that executes processes.
- Use temporary directories and in-memory fixtures; do not rely on global state.
- Integration suite runs only on a developer machine with `podman` installed
  and MUST NOT be invoked by the agent or in the sandbox.

### Fixtures
- Use `tempfile` for filesystem fixtures (if needed).
- Keep fixture trees minimal, with explicit file contents.
- Preserve legacy parity by referencing `inspiration/` for expected behavior where needed.

### Integration test suite (manual, `podman` required)
- Location: `tests/integration/` for fixtures + runner scripts.
- Runner: a thin script that runs `giftwrap` against each fixture and captures
  stdout, stderr, exit code, and the resolved runtime args.
- Each test uses simple container commands (`id`, `env`, `pwd`, `ls`, `cat`) to
  validate behavior without heavy workloads.
- Artifacts: write to `artifacts/integration/<run-id>/` in the repo so the agent
  can inspect failures later. Each case should produce:
  - `cmd.txt` (full command line), `stdout.txt`, `stderr.txt`, `exit-code`
  - `runtime-args.txt` (the computed `podman` invocation)
  - `config.json` (output of `--gw-show-config` where relevant)

#### Suggested integration cases
- Basic run: minimal config + `giftwrap -- echo ok`.
- Image override: `--gw-img` with a known small image.
- Context tag: `--gw-use-ctx` produces a tag and runs the container.
- Shares: host file visible inside container at expected mount.
- Env overrides: `GIFTWRAP_SET_*` and `GIFTWRAP_DEL_*` reflected in `env` output.
- Persisted env: round-trip across two runs using the persisted env file.
- Prelaunch hook + extra shell: verify hooks run in order with trace output.
- Git dir share: ensure `.git` is mounted when enabled.

### Incremental workflow
For each unit suite:
- Add tests and any minimal helper functions.
- Run `cargo test` as part of your loop in adding each suite without asking.
- Summarize new coverage and any behavior gaps discovered vs legacy script.
- Do not fix and suspected bugs in `giftwrap` without first prompting for
  direction.
