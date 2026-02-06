# giftwrap v2 Reimplementation Blueprint

## 1. Product Definition
`giftwrap` v2 is a Linux-only CLI that runs commands in a fast, reproducible environment described by `.giftwrap.toml`.
The environment is built from an OCI base image, mutated by a setup script, compressed to squashfs, cached, then executed with `bwrap`.

Primary goals:
1. Run commands inside an environment defined by `.giftwrap.toml`.
2. Share the project root into the sandbox at the same absolute path.
3. Run commands as host UID/GID.
4. Optimize cold-start and repeat-run latency.
5. Use `bwrap` + OCI tooling only (no `podman` backend).

Non-goals:
1. Compatibility with legacy CLI/config contracts.
2. Full Dockerfile/Containerfile parity.
3. Non-Linux support.

## 2. Required External Tools
Required binaries:
1. `bwrap`
2. `skopeo`
3. `umoci`
4. `mksquashfs`
5. `squashfuse`
6. `fusermount3` or equivalent unmount utility (`umount` fallback)

Startup behavior:
1. `giftwrap run` performs tool probes before any expensive work.
2. Missing tools produce actionable errors and exit code `2`.
3. `giftwrap run --verbose` logs detected versions once at startup.

## 3. CLI Specification
Top-level commands:
1. `giftwrap run [options] -- command ...`
2. `giftwrap print-config`
3. `giftwrap cache gc`
4. `giftwrap version`

`run` options:
1. `--rebuild`
Force rebuild even if cache exists.
2. `--print`
Print resolved build/run plan and exit without execution.
3. `--verbose`
Emit step-by-step logs to stderr.
4. `--cache-dir <path>`
Override default cache dir (`~/.giftwrap/cache`).
5. `--pull <policy>`
One of `missing`, `always`, `never`. Default `missing`.
6. `--setup-only`
Build/update cache only; do not run command.

Command requirement:
1. `giftwrap run` requires `-- command ...`.
2. If no command is provided, exit with code `2` and print:
`Error: no command specified; use 'giftwrap run -- <command ...>'`

Exit codes:
1. `0` success.
2. `1` runtime or command failure.
3. `2` usage/config/tooling validation failure.
4. `3` build pipeline failure (pull/unpack/setup/squash).
5. `4` cache lock timeout or corruption.

## 4. `.giftwrap.toml` Schema
Required keys:
1. `image` (string)
OCI image reference (tag or digest).
2. `setup_script` (string)
Path relative to build root or absolute path.

Validation rules:
1. `image` must be non-empty.
2. `setup_script` must be non-empty.
3. `setup_script` must exist before build phase starts.
4. Unknown keys are validation errors (strict schema).

Example:
```toml
image = "docker.io/library/debian:bookworm-slim"
setup_script = "giftwrap/setup.sh"
```

## 5. Build Root and Discovery
Discovery:
1. Start from current working directory.
2. Search upward for `.giftwrap.toml`.
3. First match is the build root.
4. If not found, error with exit code `2`.

Path handling:
1. All relative config paths are resolved against build root.
2. Root sharing bind mount is `build_root -> build_root`.
3. Default runtime working directory is original host cwd.

## 6. Context Hash (`ctx_sha`) Specification
Purpose:
`ctx_sha` names cache artifacts: `~/.giftwrap/cache/<ctx_sha>.sqfs`.

Hash algorithm:
1. SHA-256 over canonical input stream.
2. Input stream includes, in order:
`giftwrap-v2\0`
`config_bytes\0`
`image_ref\0`
`setup_script_sha256\0`
`context_files_manifest\0`
3. `context_files_manifest` is deterministic:
sorted list of `<relative-path>\0<file-sha256>\0<mode>\0<symlink-target?>\0`.

Manifest input files:
1. `.giftwrap.toml`
2. `setup_script`
3. If `setup_script` is a symlink, hash symlink metadata and target path.

Rationale:
1. Artifact naming stays `<ctx_sha>.sqfs` as required.
2. Hash changes when any relevant input changes.

## 7. Cache Layout and Metadata
Default cache root:
`~/.giftwrap/cache`

Layout:
1. `~/.giftwrap/cache/<ctx_sha>.sqfs`
2. `~/.giftwrap/cache/<ctx_sha>.json`
3. `~/.giftwrap/cache/locks/<ctx_sha>.lock`
4. `~/.giftwrap/cache/work/<ctx_sha>-<pid>-<timestamp>/`
5. `~/.giftwrap/cache/mnt/<ctx_sha>/`

Metadata schema (`<ctx_sha>.json`):
1. `schema_version` (u32, initial `1`)
2. `ctx_sha` (string)
3. `image_ref` (string)
4. `image_digest` (string)
5. `pull_policy_used` (string, CLI-selected)
6. `setup_script_sha256` (string)
7. `context_manifest_sha256` (string)
8. `compression` (`"lzo"`)
9. `created_unix_ms` (u64)
10. `giftwrap_version` (string)
11. `tool_versions` (object)

Cache validity:
1. `.sqfs` and `.json` both exist.
2. Metadata parses and `schema_version` supported.
3. Metadata `ctx_sha` equals requested `ctx_sha`.
4. Metadata `compression == "lzo"`.
5. If pull policy is `always`, metadata is ignored and rebuild occurs.
6. If pull policy is `missing`, reuse cache if valid.
7. If pull policy is `never`, reuse cache or fail if missing.

Atomicity:
1. Build outputs are written to `.tmp` files.
2. Rename `.tmp` to final names only after successful build.
3. Never partially overwrite valid cache artifacts.

## 8. Locking Model
Per-context lock:
1. Acquire exclusive lock on `locks/<ctx_sha>.lock` before build.
2. Lock acquisition timeout default: 10 minutes.
3. On timeout, exit code `4`.
4. Lock is held across pull, unpack, setup, squash, metadata write.

Safety:
1. Validate cache again after lock acquired (double-check).
2. On crash, lock released by OS fd close.
3. Stale work dirs are cleaned opportunistically by `cache gc`.

## 9. End-to-end Build Pipeline
Pipeline steps:
1. Discover config and build root.
2. Resolve inputs and compute `ctx_sha`.
3. Check cache validity according to pull policy.
4. If hit and not `--rebuild`, skip to runtime.
5. Acquire per-context lock.
6. Re-check cache (another process may have completed build).
7. Create work dir.
8. Pull image to OCI layout:
`skopeo copy docker://<image_ref> oci:<work>/oci:base`
9. Resolve image digest:
`skopeo inspect docker://<image_ref>` and record digest.
10. Unpack OCI:
`umoci unpack --rootless --image <work>/oci:base <work>/bundle`
11. Run setup phase (Section 10).
12. Build squashfs:
`mksquashfs <work>/bundle/rootfs <cache>/<ctx_sha>.sqfs.tmp -comp lzo -xattrs -noappend`
13. Write metadata json to `<cache>/<ctx_sha>.json.tmp`.
14. Atomically rename both temp files to final artifact paths.
15. Release lock and remove work dir.

Failure behavior:
1. Build failure keeps existing valid cache untouched.
2. Work dir retained when `--verbose` for debugging; otherwise cleaned.
3. Exit code `3`.

## 10. Setup Script Execution
Goal:
Allow deterministic rootfs mutation before squashfs creation.

Execution contract:
1. Script path resolved at host before execution.
2. Script content copied into bundle at `/tmp/giftwrap-setup.sh`.
3. Script executed inside bwrap with bundle rootfs as `/`.
4. Setup runs as uid `0` inside user namespace where available.
5. Setup receives standard variables:
`GW_BUILD_ROOT`, `GW_CTX_SHA`, `GW_IMAGE_REF`, `GW_CACHE_DIR`.

Default setup command:
`/bin/sh /tmp/giftwrap-setup.sh`

Default setup bwrap args:
1. `--die-with-parent`
2. `--new-session`
3. `--unshare-all`
4. `--share-net`
5. `--uid 0`
6. `--gid 0`
7. `--bind <bundle/rootfs> /`
8. `--proc /proc`
9. `--dev /dev`
10. `--chdir /`

Notes:
1. If host cannot create user namespace, fail with actionable message.
2. Setup script must be idempotent for stable rebuild behavior.

## 11. Runtime Execution Pipeline
Steps:
1. Ensure sqfs cache artifact exists (build if needed).
2. Mount sqfs read-only:
`squashfuse <cache>/<ctx_sha>.sqfs <cache>/mnt/<ctx_sha>`
3. Compose bwrap runtime argv.
4. Spawn bwrap child with inherited stdin/stdout/stderr.
5. Wait for child to exit, then unmount mountpoint.
6. Exit with the child exit status.

Runtime bwrap defaults:
1. `--die-with-parent`
2. `--new-session`
3. `--unshare-ipc`
4. `--unshare-pid`
5. `--unshare-uts`
6. `--unshare-cgroup-try`
7. `--share-net`
8. `--unshare-user-try`
9. `--uid <host_uid>`
10. `--gid <host_gid>`
11. `--overlay-src <cache>/mnt/<ctx_sha>`
12. `--tmp-overlay /`
13. `--proc /proc`
14. `--dev /dev`
15. `--bind <build_root> <build_root>`
16. `--chdir <host_cwd>`

Environment strategy:
1. Start with `--clearenv`.
2. Set minimal defaults:
`HOME`, `USER`, `LOGNAME`, `PATH`, `TERM` (if present on host).
3. No additional config-driven environment overrides in MVP.

Command execution:
1. `-- <argv...>` from CLI only.
2. No internal agent process required in MVP.

UID/GID matching:
1. Runtime always attempts host uid/gid inside sandbox.
2. If unsupported on host kernel config, fail fast with clear guidance.

Process lifecycle:
1. Do not `exec` bwrap directly from the top-level process in MVP.
2. Use a supervisor process to guarantee mount cleanup:
mount sqfs -> spawn bwrap child with inherited stdio -> wait -> unmount -> exit with child status.
3. Forward SIGINT and SIGTERM from supervisor to child process group.
4. Preserve child exit code exactly.

## 12. Logging and Observability
Logging channels:
1. User command stdout/stderr pass through untouched.
2. giftwrap diagnostic logs go to stderr only.

`--verbose` minimum events:
1. config discovery result
2. computed `ctx_sha`
3. cache hit/miss reason
4. tool commands executed (sanitized)
5. timings per phase (pull, unpack, setup, squash, run)

`--print` output:
1. Build root
2. Context hash
3. Cache paths
4. Setup command
5. Runtime bwrap argv
No execution should occur in `--print`.

## 13. Error Message Contract
Principles:
1. One-line primary error prefixed with `Error:`.
2. Optional follow-up `Hint:` line with remediation.
3. Include command exit status for external tool failures.

Examples:
1. `Error: required tool not found: skopeo`
2. `Error: invalid config key: workdir`
3. `Error: failed to build squashfs (exit 1)`
4. `Error: cache lock timeout for <ctx_sha>`

## 14. Rust Project Skeleton for Empty Branch
Recommended file tree:
1. `Cargo.toml`
2. `src/main.rs`
3. `src/cli.rs`
4. `src/config.rs`
5. `src/discovery.rs`
6. `src/context_hash.rs`
7. `src/tooling.rs`
8. `src/oci.rs`
9. `src/rootfs_builder.rs`
10. `src/sqfs_cache.rs`
11. `src/runtime/mod.rs`
12. `src/runtime/bwrap.rs`
13. `src/errors.rs`
14. `src/log.rs`
15. `tests/unit/*.rs`
16. `tests/manual/README.md`

Recommended dependencies:
1. `clap` (CLI parsing)
2. `serde`, `serde_json` (metadata/config)
3. `toml` (config parsing)
4. `sha2` (hashing)
5. `fs4` or `nix` (file locks)
6. `thiserror` (error enums)

No async runtime is required.

## 15. Internal API Contracts
`config.rs`:
1. `fn discover(start: &Path) -> Result<DiscoveredConfig, Error>`
2. `fn load(path: &Path) -> Result<Config, Error>`

`context_hash.rs`:
1. `fn compute(build_root: &Path, cfg: &Config) -> Result<ContextHashResult, Error>`

`sqfs_cache.rs`:
1. `fn resolve_paths(cache_root: &Path, ctx_sha: &str) -> CachePaths`
2. `fn is_valid(paths: &CachePaths, policy: PullPolicy) -> Result<bool, Error>`
3. `fn with_lock(paths: &CachePaths, timeout: Duration, f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error>`
4. `fn write_atomically(paths: &CachePaths, sqfs_tmp: &Path, meta: &CacheMetadata) -> Result<(), Error>`

`oci.rs`:
1. `fn inspect_digest(image: &str) -> Result<String, Error>`
2. `fn pull_to_layout(image: &str, layout_dir: &Path) -> Result<(), Error>`

`rootfs_builder.rs`:
1. `fn unpack(layout_dir: &Path, bundle_dir: &Path) -> Result<PathBuf, Error>`
2. `fn run_setup(rootfs: &Path, cfg: &Config, ctx_sha: &str, build_root: &Path) -> Result<(), Error>`
3. `fn build_sqfs(rootfs: &Path, output_tmp: &Path) -> Result<(), Error>`

`runtime/bwrap.rs`:
1. `fn build_argv(spec: &RunSpec) -> Vec<String>`
2. `fn exec(spec: &RunSpec) -> Result<Infallible, Error>`

`main.rs` orchestration:
1. Parse CLI.
2. Discover/load config.
3. Compute context hash and cache paths.
4. Build or reuse sqfs.
5. Build run spec.
6. Print or exec.

Core data structures:
```rust
struct Config {
    image: String,
    setup_script: PathBuf,
}

enum PullPolicy { Missing, Always, Never }

struct ContextHashResult {
    ctx_sha: String,
    manifest_sha256: String,
    manifest_entries: Vec<ManifestEntry>,
}

struct CachePaths {
    sqfs: PathBuf,
    meta: PathBuf,
    lock: PathBuf,
    mountpoint: PathBuf,
    work_root: PathBuf,
}

struct CacheMetadata {
    schema_version: u32,
    ctx_sha: String,
    image_ref: String,
    image_digest: String,
    pull_policy_used: String,
    setup_script_sha256: String,
    context_manifest_sha256: String,
    compression: String,
    created_unix_ms: u64,
    giftwrap_version: String,
    tool_versions: BTreeMap<String, String>,
}

struct RunSpec {
    host_uid: u32,
    host_gid: u32,
    build_root: PathBuf,
    workdir: PathBuf,
    mountpoint: PathBuf,
    env: BTreeMap<String, String>,
    argv: Vec<String>,
}
```

## 16. Run Algorithm (Reference Pseudocode)
```text
run():
  cli = parse_cli()
  pull_policy = cli.pull.unwrap_or(Missing)
  tools = probe_tools()
  discovered = config::discover(cwd)
  cfg = config::load(discovered.path)

  ctx = context_hash::compute(discovered.root, cfg)
  paths = sqfs_cache::resolve_paths(cache_dir(cli), ctx.ctx_sha)

  if cli.print:
    print_plan(cfg, ctx, paths)
    return 0

  if !cli.rebuild && sqfs_cache::is_valid(paths, pull_policy):
    goto runtime

  sqfs_cache::with_lock(paths.lock, timeout):
    if !cli.rebuild && sqfs_cache::is_valid(paths, pull_policy):
      goto runtime
    work = create_work_dir(paths.work_root)
    oci::pull_to_layout(cfg.image, work/oci)
    digest = oci::inspect_digest(cfg.image)
    rootfs = rootfs_builder::unpack(work/oci, work/bundle)
    rootfs_builder::run_setup(rootfs, cfg, ctx.ctx_sha, discovered.root)
    rootfs_builder::build_sqfs(rootfs, paths.sqfs.tmp)
    meta = build_metadata(cfg, ctx, digest, tools, pull_policy)
    sqfs_cache::write_atomically(paths, paths.sqfs.tmp, meta)

runtime:
  if cli.setup_only:
    return 0
  run_spec = build_run_spec(discovered.root, cwd, paths.mountpoint, cli.command_argv)
  return runtime::bwrap::run_with_mount(paths.sqfs, paths.mountpoint, run_spec)
```

## 17. Cache GC Specification
`giftwrap cache gc` behavior:
1. Delete stale work dirs older than 24h under `cache/work`.
2. Delete stale mount dirs with no active FUSE mount.
3. Delete orphan metadata files with missing `.sqfs`.
4. Delete orphan `.sqfs` files with missing metadata.
5. Keep active lock files and files modified in the last 5 minutes.
6. Optional flag `--max-age-days <n>` for pruning old valid artifacts.

Safety rules:
1. Never delete files for a `ctx_sha` if its lock is currently held.
2. Never follow symlinks while pruning.
3. Print a dry-run summary when `--print` is supplied.

## 18. Performance Plan
Must-have optimizations:
1. Avoid remote `inspect` on every run when policy is `missing` and cache is valid.
2. Avoid rebuilding when setup/context unchanged.
3. Use atomic rename instead of copying final artifacts.
4. Keep runtime command composition allocation-light.
5. Keep log formatting lazy unless `--verbose`.

Optional optimizations after MVP:
1. Persistent `squashfuse` mount pool.
2. Parallel hashing of context files.
3. Build metrics cache for last N runs.

## 19. Test Plan (Sufficient for Reimplementation)
Unit tests:
1. CLI parse matrix for all options and delimiters, including error on missing `-- command ...`.
2. Config validation success/failure cases.
3. Discovery upward search behavior.
4. Context hash determinism and change sensitivity.
5. Cache metadata parse and validity rules.
6. Lock timeout and lock reentry behavior.
7. Build command composition (`skopeo`, `umoci`, `mksquashfs`).
8. Runtime bwrap argv composition.

Manual integration tests (developer machine only):
1. Cold run from uncached image.
2. Warm run cache hit.
3. `--rebuild` forces rebuild.
4. Setup script modifies rootfs and effect visible at runtime.
5. Host UID/GID visible in sandbox.
6. Build root path shared exactly.
7. Failures for missing tools and bad setup script.

Artifact collection for manual tests:
1. `artifacts/manual/<run-id>/stdout.txt`
2. `artifacts/manual/<run-id>/stderr.txt`
3. `artifacts/manual/<run-id>/exit-code.txt`
4. `artifacts/manual/<run-id>/plan.txt` (`--print` output)

## 20. Implementation Sequence for Empty Branch
Phase 1:
1. Bootstrap crate, CLI, error types, logging.
2. Implement config discovery/load/validation.

Phase 2:
1. Implement context hash + cache path resolution.
2. Implement metadata read/write and lock primitives.

Phase 3:
1. Implement OCI inspect/pull wrappers.
2. Implement unpack + setup + squash build pipeline.

Phase 4:
1. Implement runtime sqfs mount/unmount helpers.
2. Implement bwrap run argv and exec path.

Phase 5:
1. Wire main orchestration and `--print`.
2. Add cache gc command.
3. Complete tests and manual fixtures.

Definition of done:
1. All unit tests pass.
2. Manual integration checklist passes.
3. README documents v2 CLI and config.
4. This plan and implementation match with no undefined behavior gaps.

## 21. Migration Note
This rewrite is intentionally breaking.
Carry a short migration doc:
1. legacy flags removed
2. new `.giftwrap.toml` schema
3. setup script model replacing Containerfile semantics
4. new cache and runtime behavior
