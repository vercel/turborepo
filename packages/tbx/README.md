# `@turbo/tbx`

Repo-local helper for working on Turborepo in Vercel Sandboxes.

## Quickstart

Install dependencies for this package:

```bash
pnpm install --frozen-lockfile --ignore-scripts --filter @turbo/tbx
```

Log in to Vercel Sandbox:

```bash
pnpm tbx login
```

Log in to GitHub on the host. `tbx` uses this host-side token only for Sandbox firewall credential brokering:

```bash
gh auth login
```

Link the `@turbo/tbx` package to the Vercel project that should own your sandboxes:

```bash
cd packages/tbx
vercel link
```

- Vercel users should link to a specific project. If you work at Vercel and want to use this tooling, please ask the Turborepo team what you should link to.
- External contributors should link to a Vercel project under their own account or team of their choosing.

For Vercel AI Gateway access, put `VERCEL_OIDC_TOKEN` in `packages/tbx/.env.local`. `tbx` brokers it through the Sandbox firewall instead of writing the real token into task sandboxes.

Create or refresh the warm base sandbox for the current `origin/main` SHA:

```bash
pnpm tbx base refresh
```

After the warm base exists, mapped Vercel users can refresh only dotfiles in that base:

```bash
pnpm tbx base refresh --dotfiles
```

Start work on a branch sandbox:

```bash
pnpm tbx new cache-fix
pnpm tbx sh cache-fix
```

`new` creates `turbo-cache-fix` from the current warm base snapshot, initializes the matching branch, configures brokered credentials, and enables verified commit signing. `sh` connects to that existing sandbox.

You can also run `pnpm tbx sh cache-fix` directly. If the sandbox does not exist, `tbx` logs that it is creating it from the warm base snapshot first.

## Commands

```bash
pnpm tbx setup
```

Ensures the repo-pinned Sandbox CLI dependency is installed and prints current auth/project context.

```bash
pnpm tbx login
```

Runs `sandbox login` through the repo-pinned Sandbox CLI.

```bash
pnpm tbx auth
```

Shows the Sandbox CLI version, host GitHub auth availability, Vercel OIDC availability, and the Vercel project context visible to `tbx`.

```bash
pnpm tbx ls
```

Lists `turbo-*` sandboxes in the current Sandbox CLI project context.

```bash
pnpm tbx creds github <name>
```

Applies credential brokering to `turbo-<name>`. `tbx` also does this automatically before `new`, `sh`, and `run` complete.

```bash
pnpm tbx creds check <name>
```

Verifies brokered GitHub auth and Vercel provider detection inside `turbo-<name>` without exposing host tokens.

```bash
pnpm tbx base refresh
```

Creates or refreshes `turbo-base-<origin-main-sha12>`, installs `turbo@latest` globally so it is on `PATH`, installs Turborepo dependencies, runs `cargo build`, stops the sandbox, and snapshots it.

For mapped Vercel users, the base name includes the username:

```text
turbo-base-<vercel-username>-<origin-main-sha12>
```

Use `--dotfiles` to refresh only mapped user dotfiles in an existing base, then snapshot it. This does not reinstall system packages, run `pnpm install`, or run `cargo build`:

```bash
pnpm tbx base refresh --dotfiles
```

```bash
pnpm tbx base id
```

Prints the current base sandbox name and snapshot ID when available.

```bash
pnpm tbx new <name>
```

Creates `turbo-<name>` from the newest available base snapshot, applies credential brokering, initializes a matching branch, and configures verified commit signing. If that base is older than current `origin/main`, `tbx` warns but continues.

```bash
pnpm tbx sh <name>
```

Opens an interactive login Bash shell in `turbo-<name>`. Creates it first if missing.

```bash
pnpm tbx run <name> -- <command>
```

Runs a command in `/vercel/sandbox/src/turbo` inside `turbo-<name>`. Creates it first if missing.

```bash
pnpm tbx stop <name>
```

Stops the current session for `turbo-<name>`. Persistent sandbox state is saved by Vercel.

```bash
pnpm tbx rm <name>
```

Permanently removes `turbo-<name>`.

## Defaults

Base and task sandboxes are created with:

```text
Runtime: node22
vCPUs: 32
Memory: 64 GiB, derived by Vercel from 32 vCPUs
Timeout: 30m, the current Sandbox API maximum
Task snapshot expiration: 14d
Base snapshot expiration: none
Repo path: /vercel/sandbox/src/turbo
```

## Notes

`tbx` does not store local snapshot IDs or project overrides. Base sandboxes are named from `origin/main` SHAs, and task sandboxes are named from your task name. New task sandboxes use the newest existing base snapshot and warn when that base is behind current `origin/main`.

Project and account resolution are owned by the Sandbox CLI and normal Vercel project context.

Dotfiles are installed only during `base refresh --dotfiles`, not per task sandbox. Task sandboxes inherit dotfiles from the base snapshot.

## Credential Brokering

Task sandboxes use Sandbox firewall credential brokering for GitHub and Vercel OIDC. The real GitHub token is resolved on the host with `gh auth token`, and the real Vercel OIDC token is loaded from `packages/tbx/.env.local`. Both are sent to the Sandbox API as network policy transforms and never written into the sandbox filesystem.

Inside `sh` and `run`, `tbx` sets dummy GitHub and Vercel auth environment values so tools send auth headers. The firewall replaces those dummy headers with host credentials while requests leave the sandbox.

Do not run `gh auth login` inside a sandbox. Use `pnpm tbx creds check <name>` to verify brokered auth.

The task sandbox firewall is custom and deny-by-default. GitHub domains required by `gh` and HTTPS Git are allowed, along with Vercel domains required for OIDC-backed provider access. Other outbound domains are blocked unless explicitly added to the policy in `tbx`.

Credential brokering keeps tokens out of the sandbox, but the sandbox can still exercise whatever permissions the host credentials have against allowed domains. Use the least-privileged host auth suitable for the work.

Task sandboxes are created with the brokered network policy already attached. `tbx` also strips credentials from the local Git remote before cloning during base refresh so host credential-bearing remote URLs are not persisted into sandbox snapshots.

## Verified Commits

`tbx` configures verified commits automatically for task sandboxes using a host-backed signing broker.

The host signing key stays on the host. The sandbox gets a Git SSH signing shim, but no signing key. When Git signs a commit, the shim writes the commit signing payload into `/tmp/tbx-sign` in the sandbox. A host broker running during `pnpm tbx sh` or `pnpm tbx run` watches that directory through the Sandbox SDK, signs the payload on the host, and writes the signature back into the sandbox filesystem.

No sandbox signing key is generated, and no GitHub signing key is registered per sandbox. The public signing key must already be trusted by GitHub for verified commits.
