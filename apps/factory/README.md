# Turborepo Agents

The operator page and its API routes rely on Vercel Deployment Protection for access control. Keep Deployment Protection enabled for every deployed environment that exposes them.

## Harness maintenance

Manual daily-example maintenance runs with `HarnessAgent`. Operators can choose Claude Code, Codex, or OpenCode; each runs in an isolated Vercel Sandbox. Performance runs and scheduled Eve automation are unchanged.

Configure `GITHUB_TOKEN_EXCHANGE_URL`. The exchange endpoint receives Vercel OIDC bearer authentication and must return `{ "token": string, "expires_at": string }` for the requested `vercel/turborepo` write permissions. Vercel OIDC authenticates Vercel Sandbox and AI Gateway. GitHub authorization is injected by the sandbox network policy and is not exposed to agent processes.

The workflow clones Turborepo into the selected sandbox and runs the chosen official Harness SDK adapter there. The adapter and sandbox registries in `agent/lib/harness-agent.ts` are independent, so another provider can be added without changing the workflow or run API.

## Unified run dashboard

Connect a private Vercel Blob store to the project to enable the operator page's unified run dashboard. The recommended OIDC configuration provides `BLOB_STORE_ID` and `VERCEL_OIDC_TOKEN`; `BLOB_READ_WRITE_TOKEN` can provide local ledger access, but Harness execution and Sandbox inventory still require Vercel OIDC.

Eve lifecycle hooks and Harness workflow steps write the same normalized ledger containing the latest 100 runs by start time. Collection begins after this version is deployed; it does not backfill older Agent Runs. The page polls that ledger and displays the eight most recently updated `ai-sdk-harness*` resources from the Vercel Sandbox API. Detailed transcripts remain in Agent Runs for Eve and Workflow observability for Harness.

An Eve run's model is recorded when its first model step starts rather than when the session starts. The agent selects its author model dynamically, so `session.started` carries no model id and the ledger fills the field from `step.started` instead.
