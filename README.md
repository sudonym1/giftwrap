# giftwrap v2

`giftwrap` v2 is a Linux-only CLI that runs commands in a reproducible sandbox defined by `.giftwrap.toml`.

## Required tools

- `bwrap`
- `skopeo`
- `umoci`
- `mksquashfs`
- `squashfuse`
- `fusermount3` (or `umount` fallback)

## Config

Create `.giftwrap.toml` at your project root:

```toml
image = "docker.io/library/debian:bookworm-slim"
setup_script = "giftwrap/setup.sh"

[env]
PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/opt/custom/bin"
```

Schema is strict: only `image`, `setup_script`, and `env` are allowed.

## Commands

- `giftwrap run [options] -- <command ...>`
- `giftwrap print-config`
- `giftwrap cache gc [--print] [--max-age-days <n>]`
- `giftwrap version`

`giftwrap run` options:

- `--rebuild`
- `--print`
- `--verbose`
- `--cache-dir <path>`
- `--pull <missing|always|never>`
- `--setup-only`

## Exit codes

- `0` success
- `1` runtime/command failure
- `2` usage/config/tooling validation failure
- `3` build pipeline failure
- `4` cache lock timeout or cache corruption

## Notes

- Cache artifacts live under `~/.giftwrap/cache` by default.
- Cache artifact names are derived from a deterministic context hash.
- Runtime binds the discovered build root to the same absolute path inside the sandbox.
- `env` variables are applied to runtime command execution only.
