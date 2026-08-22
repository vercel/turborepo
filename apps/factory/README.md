# Turborepo Agents

The operator page and its API routes rely on Vercel Deployment Protection for access control. Keep Deployment Protection enabled for every deployed environment that exposes them.

## Start work from the operator page

The two scheduled operations run a fixed prompt. "Start work" is the ad-hoc
path: it opens an ordinary durable Eve session from the browser, on the same
factory image and the same checkout of `main`, and the operator drives it by
typing. Requests are not scoped to `examples/` and no pull request happens on
its own — `create_pull_request` runs under the operator's approval, which the
chat renders as a prompt with the tool's branch and title in it. Answer it and
the draft pull request is pushed; decline it and nothing is.

The chat talks to the Eve session routes directly through `useEveAgent`, so
`agent/channels/eve.ts` has an authenticator for it. Deployment Protection still
decides who reaches the deployment; the `operatorConsole()` entry only
establishes that a request came from the console page. It requires the
`x-operator-action: open-operator-chat` marker the page attaches to every Eve
request — those routes send no CORS headers, so another site cannot get a custom
header past a preflight — and rejects anything announcing a cross-site fetch or
naming an origin other than the host the browser addressed. It reads that host
from `x-forwarded-host`, because both Vercel and `next start` route these paths
to the Eve service through a proxy that rewrites the host it dials.

The session's principal is a _user_, not the app principal the schedules use,
which is what keeps the approval gate on and the automated example and
performance scope gates off.

The thread's session cursor and event log live in browser storage, so a reload
lands back in the same conversation. A turn that is still running when the page
reloads keeps running: its transcript is in Agent Runs, and "New chat" starts a
fresh session.

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

A push to `main` reaches `POST /api/github/push` through a Vercel Connect
trigger. Connect verifies GitHub's signature, the route verifies Connect's
Vercel OIDC credential, and then calls the same application function as the
operator button and the `rebuild_factory_image` Eve tool. That function creates
a build sandbox and detaches the provisioning script inside it. Dashboard
polling and a one-minute Eve schedule reconcile its markers, snapshot the
result, and publish the snapshot id as the current image. No GitHub Actions job
or model call is involved. When a published image already exists for the same
toolchain the build boots from it, so a merge build only has to fast-forward
the checkout, refresh dependencies, and recompile.

Rapid merges are resolved in the ledger rather than by racing: claiming a
build cancels every build still in flight, marks it superseded, and deletes its
sandbox. Each reconciliation re-reads the ledger before doing work and a build
that has lost can neither report progress nor publish, so only the newest
revision on `main` is ever published.
`tests/factory-image-ledger.test.mjs` covers those transitions.

Configure it with:

- A private Vercel Blob store (the ledger lives beside the run
  registry).
- A GitHub Vercel Connect connector subscribed to `push`.
- `FACTORY_IMAGE_CONNECTOR_ID` set to that connector's stable `scl_...` ID.
- A Production trigger destination for the `turborepo-factory` project at
  `/api/github/push`. Because Deployment Protection covers that path, append
  the automation bypass token as the `x-vercel-protection-bypass` query
  parameter. Connect authenticates forwarded requests with Vercel OIDC, and
  the route requires its signed connector ID; direct GitHub webhooks and
  other same-project OIDC callers are rejected. Other events are acknowledged
  and ignored.

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

## Agent Runs

The operator page links to Vercel Agent Runs, the canonical record of scheduled jobs and operator chats. Do not use the local Blob run registry as an audit trail; it exists only to coordinate Harness execution.

Detailed transcripts remain in Agent Runs for Eve and Workflow observability for Harness.

An Eve run's model is recorded when its first model step starts rather than when the session starts. The agent selects its author model dynamically, so `session.started` carries no model id and the ledger fills the field from `step.started` instead.
