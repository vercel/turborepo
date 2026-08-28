# Turborepo Agents

The operator page and its API routes rely on Vercel Deployment Protection for access control. Keep Deployment Protection enabled for every deployed environment that exposes them.

## Workspaces

"Start work" creates a durable Factory workspace driven by
[`fx`](https://fx.sh/). Each workspace has one server-side record, named Vercel
Sandbox, fx session, transcript, and shareable `/workspaces/<id>` URL. A
Workflow advances the same saved fx session one turn at a time, so another
browser or operator can reopen the URL and continue the same conversation and
checkout.

The workspace page can load the current Git status and a capped diff on demand,
and exposes a browser terminal, the `sandbox ssh <name>` command, and the exact
`fx resume --id <session>` command for rejoining the chat after connecting.
Workflow observability remains the full execution audit. When fx reports a
Turborepo pull request URL, the workspace records and links it.

The Eve GitHub channel automatically handles newly opened public issues. A
separate tool-less security subagent first reviews the initial issue content for
prompt injection, reproduction tampering, and other suspicious behavior. Any
suspicious signal fails closed: Factory does not inspect or run the reproduction
and posts a Slack alert with a threaded explanation. Passed issues receive a
confidence assessment. Low- and medium-confidence issues send a Slack alert
with a threaded rationale and get an investigation report only. Only
high-confidence issues proceed to a focused fix, validation, and a draft
`agents/issue-*` pull request.

The channel also follows Factory-created pull requests whose head is an
`agents/*` branch. Timeline and inline review comments from collaborators with
write access start a turn without requiring an `@mention`. The turn checks out
the current PR head, replies in the same GitHub thread, and can publish validated
feedback changes back to that exact branch. Bot, external-user, non-PR, and
non-Factory-branch comments fail closed and are ignored.

Workspace records live as private `factory-workspaces/v1/<id>.json` Blob
objects. Mutation routes require an exact same-origin request and action header;
Vercel Deployment Protection remains the outer operator authentication layer.
The durable Eve event stream is the transcript and execution activity source.
Storage requires either `BLOB_READ_WRITE_TOKEN`, or both `BLOB_STORE_ID` and
`VERCEL_OIDC_TOKEN`.

Sandbox Drives are currently private beta. Factory defaults to Eve's regular
session sandbox storage so workspace creation still works without beta access.
After the Vercel team is enrolled, set `FACTORY_WORKSPACE_DRIVES=1` to mount a
per-session Drive and persist the checkout across replacement sandbox compute.

### Local terminal

Set `FACTORY_URL` to the protected deployment. For automation-protected
deployments, also set `VERCEL_AUTOMATION_BYPASS_SECRET`.
Install and authenticate the Vercel `sandbox` CLI before using `factory ssh`.

```sh
pnpm --filter examples-agent factory list
pnpm --filter examples-agent factory start "Investigate the affected warning and open a PR"
pnpm --filter examples-agent factory ssh ws_...
```

The SSH command prints the exact fx resume command before connecting. The same
workspace remains available from the web while the local terminal is attached.

## Factory image

Every agent in this app runs against the same sandbox base layer, the
factory image: a Turborepo checkout plus everything `cargo build` and
`pnpm test` need. `agent/lib/factory-image.ts` is the single source of
truth for it — pinned versions, the shell that installs them, and the
fingerprint that decides when a rebuild is required. It installs the
system build toolchain (`build-essential`, `pkg-config`, `lld`, OpenSSL
headers, `jq`, `zstd`, `gh`), Cap'n Proto, `protoc`, Zig, Node.js, pnpm, fx, the
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
- A GitHub Vercel Connect connector subscribed to `push` and
  `pull_request_review_comment`, with pull-request read/write, contents write,
  and repository collaborator metadata read permissions. Route `push` to
  `/api/github/push`, and route `pull_request_review_comment` to `/eve/v1/github`
  (including the Deployment Protection bypass query parameter on both
  destinations).
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
fast-forwards its checkout to the current `main`. New fx workspaces do the same,
and provision the shared image phases before their first turn when no matching
image exists. Resumed workspaces preserve their checkout, fx session, and
uncommitted changes.

A toolchain change provisions the template from scratch during the next
Vercel build, because Eve prewarms sandbox templates there. Measured
against `vercel/eve:latest`, every phase through verification takes about
two minutes, and the phases that compile Rust are wrapped in timeouts so
one bad upstream release cannot hold a deployment build open. Only the
merge webhook asks for the warm `cargo build`, which runs off the
deployment path inside the build sandbox.

Configure `GITHUB_TOKEN_EXCHANGE_URL`. The exchange endpoint receives Vercel OIDC bearer authentication and must return `{ "token": string, "expires_at": string }` for the requested `vercel/turborepo` write permissions. Vercel OIDC authenticates Vercel Sandbox and AI Gateway. GitHub authorization is injected by the sandbox network policy and is not exposed to agent processes.

## Agent Runs

The operator page links to Vercel Agent Runs, the audit record for Eve schedules.
fx workspace turns are audited through Workflow observability. Workspace Blob
records hold the resumable UI transcript and control-plane state, not the
complete execution audit.

An Eve run's model is recorded when its first model step starts rather than when the session starts. The agent selects its author model dynamically, so `session.started` carries no model id and the ledger fills the field from `step.started` instead.
