# Turborepo Agents

The operator page and its API routes rely on Vercel Deployment Protection for access control. Keep Deployment Protection enabled for every deployed environment that exposes them.

## Factory image

Every agent in this app runs against the same sandbox base layer, the
factory image: a Turborepo checkout plus everything `cargo build` and
`pnpm test` need. `agent/lib/factory-image.ts` is the single source of
truth for it — pinned versions, the shell that installs them, and the
fingerprint that decides when a rebuild is required. It installs the
system build toolchain (`build-essential`, `pkg-config`, `lld`, OpenSSL
headers, `jq`, `zstd`), Cap'n Proto, `protoc`, Zig, Node.js, pnpm, the
`rust-toolchain.toml` nightly with `rustfmt` and `clippy`, the workspace
`node_modules`, a warm Cargo registry, and the `hyperfine`,
`cargo-bloat`, and `twiggy` tools the performance skill reaches for. The
last phase verifies each tool is present and writes a version manifest.

`tests/factory-image.test.mjs` fails when those pins drift from
`rust-toolchain.toml`, the root `package.json`, the CI setup actions, or
`.devcontainer/Dockerfile`, so the local dev container and the agents'
sandbox stay on one toolchain.

### Rebuilding on every merge

A push to `main` reaches `POST /api/github/push`, which verifies the
GitHub HMAC signature and starts the `factory-image` workflow. The
workflow creates a build sandbox, detaches the provisioning script inside
it, polls the markers the script writes, snapshots the result, and
publishes the snapshot id as the current image. No GitHub Actions job is
involved. When a published image already exists for the same toolchain
the build boots from it, so a merge build only has to fast-forward the
checkout, refresh dependencies, and recompile.

Rapid merges are resolved in the ledger rather than by racing: claiming a
build cancels every build still in flight, marks it superseded, stops its
workflow run, and deletes its sandbox. Each step re-reads the ledger
before doing work and a build that has lost can neither report progress
nor publish, so only the newest revision on `main` is ever published.
`tests/factory-image-ledger.test.mjs` covers those transitions.

Configure it with:

- A private Vercel Blob store (the ledger lives beside the run
  registry).
- `FACTORY_IMAGE_WEBHOOK_SECRET` — the webhook secret. Falls back to
  `GITHUB_WEBHOOK_SECRET`, the secret the Eve GitHub channel already
  verifies with, when the same GitHub App also subscribes to `push`.
- A webhook delivering `push` to `https://<deployment>/api/github/push`.
  Deployment Protection covers that path, so append the automation
  bypass token as a query parameter
  (`?x-vercel-protection-bypass=<secret>`); the HMAC signature is what
  authenticates the delivery. Other events are acknowledged and ignored.

The operator page shows the published image, the toolchain fingerprint,
and recent builds, and can start a build for the current `main` head with
`POST /api/factory-image`.

### How the image is consumed

`agent/sandbox.ts` builds the Eve sandbox template from the same phases.
Eve freezes `revalidationKey` at build time, so the template rotates when
the toolchain fingerprint changes or a newer image is published, and
boots from the published snapshot when one matches. Each session then
fast-forwards its checkout to the current `main`. Harness sessions do the
same through `sandboxConfig` in `agent/lib/harness-agent.ts`, and fall
back to a shallow clone on a stock runtime when no image matches this
deployment's toolchain.

A toolchain change provisions the template from scratch during the next
Vercel build, because Eve prewarms sandbox templates there. Measured
against `vercel/eve:latest`, every phase through verification takes about
two minutes, and the phases that compile Rust are wrapped in timeouts so
one bad upstream release cannot hold a deployment build open. Only the
merge webhook asks for the warm `cargo build`, which runs off the
deployment path inside the build sandbox.

## Harness maintenance

Manual daily-example maintenance runs with `HarnessAgent`. Operators can choose Claude Code, Codex, or OpenCode; each runs in an isolated Vercel Sandbox. Performance runs and scheduled Eve automation are unchanged.

Configure `GITHUB_TOKEN_EXCHANGE_URL`. The exchange endpoint receives Vercel OIDC bearer authentication and must return `{ "token": string, "expires_at": string }` for the requested `vercel/turborepo` write permissions. Vercel OIDC authenticates Vercel Sandbox and AI Gateway. GitHub authorization is injected by the sandbox network policy and is not exposed to agent processes.

The workflow clones Turborepo into the selected sandbox and runs the chosen official Harness SDK adapter there. The adapter and sandbox registries in `agent/lib/harness-agent.ts` are independent, so another provider can be added without changing the workflow or run API.

## Unified run dashboard

Connect a private Vercel Blob store to the project to enable the operator page's unified run dashboard. The recommended OIDC configuration provides `BLOB_STORE_ID` and `VERCEL_OIDC_TOKEN`; `BLOB_READ_WRITE_TOKEN` can provide local ledger access, but Harness execution and Sandbox inventory still require Vercel OIDC.

Eve lifecycle hooks and Harness workflow steps write the same normalized ledger containing the latest 100 runs by start time. Collection begins after this version is deployed; it does not backfill older Agent Runs. The page polls that ledger and displays the eight most recently updated `ai-sdk-harness*` resources from the Vercel Sandbox API. Detailed transcripts remain in Agent Runs for Eve and Workflow observability for Harness.
