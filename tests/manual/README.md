# Manual Integration Test Checklist

Run these on a Linux machine with required tools installed.

## Suggested run-id artifact pattern

- `artifacts/manual/<run-id>/stdout.txt`
- `artifacts/manual/<run-id>/stderr.txt`
- `artifacts/manual/<run-id>/exit-code.txt`
- `artifacts/manual/<run-id>/plan.txt`

## Scenarios

1. Cold run from uncached image.
2. Warm run with cache hit.
3. `--rebuild` forcing rebuild.
4. Setup script rootfs mutation visible at runtime.
5. Host UID/GID matching inside sandbox.
6. Build root bind mounted at exact host path.
7. Missing tool and invalid setup script failure paths.

## Helpful commands

```bash
giftwrap run --print -- /bin/true > artifacts/manual/<run-id>/plan.txt
giftwrap run -- /usr/bin/id
```
