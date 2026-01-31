# Giftwrap integration suite (manual)

This suite runs real containers with `podman` and must only be run on a
developer machine. Do not run it in the sandbox.

## Prereqs
- `podman` installed and working.
- `giftwrap` binary built (recommended: `cargo build --release`).
- `cargo` installed (runner will build the musl agent if needed).

## Run
```bash
GIFTWRAP_INTEGRATION=1 bash tests/integration/run.sh
```

Optional environment variables:
- `GIFTWRAP_BIN`: path to the `giftwrap` binary to execute.
- `GW_AGENT_BIN`: path to a container-compatible `giftwrap` binary (static/musl).
- `GW_IMAGE`: base image used by fixtures (default: `docker.io/library/debian:bookworm-slim`).
- `GW_IMAGE_ALT`: image used by the image-override case (default: `docker.io/library/debian:bookworm`).
- `RUN_ID`: override the artifacts run id (default: UTC timestamp).

Note: the default image includes `/bin/bash` for the extra-shell case. If you
override `GW_IMAGE`, ensure the image has `bash` and user-management tools.
The runner will refuse to start unless it can find a static/musl giftwrap
binary for the in-container agent (`GW_AGENT_BIN`).

Run a single case:
```bash
GIFTWRAP_INTEGRATION=1 bash tests/integration/run.sh --case basic-run
```

## Artifacts
Each run writes artifacts under `artifacts/integration/<run-id>/`:
- `cmd.txt` (command line)
- `stdout.txt`, `stderr.txt`, `exit-code`
- `runtime-args.txt` (from `--gw-print`)
- `config.json` (from `--gw-show-config`)

## Cases
- `basic-run`: minimal config + `echo ok`.
- `image-override`: `--gw-img={{GW_IMAGE_ALT}}` override.
- `context-tag`: `.gwinclude` + `--gw-use-ctx={{CTX_SHA}}`.
- `shares`: `extra_shares` mount verification.
- `env-overrides`: config override via `GW_USER_OPT_SET_env_overrides` (legacy `GIFTWRAP_SET_*`).
- `persist-env`: persisted environment round-trip across two runs.
- `prelaunch-extra-shell`: prelaunch hook + extra shell + prefix cmd.
- `git-dir-share`: `share_git_dir` mount of the repo git dir.
- `rebuild-flag`: `--gw-rebuild` builds a local image and reads a marker file.
