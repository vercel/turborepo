# Graceful Shutdown Repros

Tiny repos for manually trying shutdown behavior with the locally built `turbo`
binary.

Build `turbo` once from the workspace root:

```sh
CARGO_INCREMENTAL=0 cargo build -p turbo --bin turbo
```

Then try any of these:

1. `01-graceful-once`
2. `02-force-twice`
3. `03-noninteractive-timeout`
4. `04-sigkill-parent-death`
5. `05-node-signals`
6. `06-multi-app-signals`
7. `07-docker-signals`

Each repo has its own `README.md` with exact commands.

Before rerunning a repro, clear any ignored marker files from that repro
directory:

```sh
git clean -fdX .
```

These are intentionally tiny no-install repros, so you may see warnings about:

- no locally installed `turbo`
- a missing lockfile

That is expected for these scratch repos.
