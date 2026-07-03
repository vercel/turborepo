# turborepo-sccache-shim

**Experimental spike — not shipped, not wired into releases.**

A localhost HTTP server that speaks the subset of WebDAV that sccache's
`SCCACHE_WEBDAV_ENDPOINT` backend (opendal) uses, translating it to the
Turborepo Remote Cache artifacts API (`/v8/artifacts/{hash}`).

## What it proves

sccache can use Turborepo Remote Cache as its compile-cache storage **with
zero sccache modifications**. This is the Tier-2 remote caching story for
native Cargo support: turbo runs this shim locally, points
`RUSTC_WRAPPER=sccache` at it, and every rustc invocation is remote-cached
using the user's existing turbo credentials — compile-level caching beneath
turbo's task-level caching.

## Measured results (local spike against turborepo-vercel-api-mock)

Workload: `turbo build --filter=turbo-trace --force` with a scratch
`CARGO_TARGET_DIR` (166 cacheable compile units of 214 total; the
uncacheable remainder are bins, build scripts, and proc-macros, which
sccache cannot cache by design).

| Scenario | Wall time | CPU time |
| --- | --- | --- |
| Cold build, no sccache (baseline) | 21.6s | 99.6s |
| Cold build, populating through shim | 26.2s (+21%) | — |
| Cold target, warm remote through shim | **12.0s (1.8x)** | 22.8s (**4.4x**) |
| (Reference: warm local-disk sccache, no shim) | 13.5s | 22.8s |

Hit rate on rebuild: **166/166 (100%)**. The shim adds no measurable
overhead versus sccache's local disk backend.

Artifact profile (the go/no-go data for the artifacts API):

- 166 artifacts, 95.5 MiB total
- min / median / p90 / p99 / max: ~0 / 195 KiB / 1.5 MiB / 6.8 MiB / 7.0 MiB
- Request profile per cold-populate + warm-rebuild cycle: 334 GET, 168 PUT,
  168 PROPFIND, 0 MKCOL

Extrapolating to the full `turbo` binary closure (~700 units): roughly
400–500 MiB and ~700 artifacts per toolchain/platform/profile combination.

## Protocol notes (discovered empirically, sccache 0.16 / opendal 0.55)

- Keys arrive as `{prefix}/{a}/{b}/{c}/{hash}` (sccache shards by the key's
  first three characters); the final segment is the full key and becomes
  the flattened artifact hash, prefixed with `sccache-` to namespace away
  from turbo's task artifacts.
- opendal stats paths with `PROPFIND` (`Depth: 0`) before reads and writes.
  Its 0.55 parser requires `getlastmodified` to be present in the
  multistatus response — it is a non-optional field — so the shim always
  emits one.
- The write path stats the parent directory first; reporting every
  directory as an existing collection satisfies it (the artifact namespace
  is flat), and `MKCOL` is then never issued.
- A `.sccache_check` key is written and read at server startup to probe
  read/write access. If the write probe fails, sccache silently enters
  read-only mode — misconfigurations degrade to "no caching", not errors.

## Operational findings

- **Strict env mode strips sccache configuration.** `SCCACHE_WEBDAV_*` must
  reach the cargo process. In the spike this required `--env-mode=loose`;
  the real feature should inject these variables into cargo task
  environments directly when the shim is active, sidestepping passthrough
  configuration entirely.
- `CARGO_INCREMENTAL=0` is required (sccache cannot cache incremental
  compilation). Right for CI; locally, cargo's incremental cache is usually
  the better trade — guidance is sccache in CI, incremental for local dev.
- `RUSTC_WRAPPER` participates in turbo's cargo task hashes, so runs with
  and without sccache do not share turbo task caches (conservative,
  intentional).

## Running it

```sh
# Terminal 1: a Remote Cache API (mock shown; any conforming API works)
turborepo-vercel-api-mock 8701

# Terminal 2: the shim
SHIM_PORT=8702 SHIM_UPSTREAM=http://127.0.0.1:8701 \
SHIM_TOKEN=expected_token SHIM_TEAM_ID=expected_team_id \
turborepo-sccache-shim

# Terminal 3: build through it
SCCACHE_WEBDAV_ENDPOINT=http://127.0.0.1:8702 \
SCCACHE_WEBDAV_TOKEN=expected_token \
RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 \
TURBO_EXPERIMENTAL_CARGO=1 \
turbo build --filter=<crate> --env-mode=loose
```
