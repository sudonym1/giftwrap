# giftwrap v2 Migration Notes

`giftwrap` v2 is intentionally breaking.

## What changed

1. Legacy flags were removed.
2. Configuration is now strict `.giftwrap.toml` with only:
   - `image`
   - `setup_script`
   - `env` (optional table of env vars passed to runtime command execution)
3. Containerfile-style mutation is replaced by setup-script execution inside a build rootfs.
4. Cache and runtime behavior changed:
   - per-context squashfs artifacts under `~/.giftwrap/cache`
   - lock-based build coordination
   - bwrap runtime with project-root bind mount at the same absolute path
