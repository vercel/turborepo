---
description: "Use when maintaining Turborepo examples: auditing or updating package versions, packageManager pins, Node engines, README commands, turbo tasks, lockfiles, or validating that examples still build, lint, test, and typecheck."
---

# Turborepo Examples Maintenance

Use this procedure for any request to inspect, update, modernize, validate, or repair files under `examples/`.

## Scope

- Treat examples as user-facing templates. Prefer simple, modern, copyable patterns over clever abstractions.
- Preserve package-manager intent. Do not convert an npm, pnpm, Yarn, or Berry example to another manager unless asked.
- Keep changes minimal except where latest-version migrations require broader code, config, or tooling changes.
- Do not add compatibility shims as a way to avoid migration. Prefer real migration, package replacement, or configuration changes that make latest versions work cleanly.
- If the user asks broadly to update examples, fan the work out one example at a time instead of updating them all in one run. Do not ask for scoping or risk tolerance.
- Within the example you own, update every versioned value. Handle a large upgrade in internal batches, and do not stop at the audit phase or at a checkpoint summary.
- Never ask the user questions. If a decision would normally require input, choose the path that gets the example to exact latest stable pins and continue.
- Do not produce interim reports as a substitute for finishing.

## Per-Example Fan-Out

A broad update request is dispatch work, not update work. One run must never walk the whole `examples/` directory.

1. Call `find_stale_examples`. It returns `updateQueue` (stale, no open pull request) and `skipped` (already covered by an open pull request, updated within the window, or missing commit history).
2. An example is stale when `examples/<name>` has no commit in the last week. Keep the default seven-day window unless the user names a different one.
3. Use the `Workflow` tool to dispatch exactly one `tools.agent` call per `updateQueue` entry, oldest example first. Fan the calls out with `Promise.all` so they run concurrently, and keep the fan-out inside the program's subagent budget by chunking when the queue is larger than the budget.
4. Give each call everything it needs, because a child never sees this run's history: the example name, its `lastCommitAt`, the branch to use (`agents/examples/<example>`), and the instruction to update only that example and open one pull request for it.
5. Do not read, write, validate, or open a pull request for an example yourself in a fan-out run. Dispatch, collect the child results, and report.
6. Report the dispatched examples, each child's outcome, and the skipped examples with their reasons.

An example of the program shape, where `queue` comes from the tool result:

```js
const results = await Promise.all(
  queue.map((entry) =>
    tools.agent({
      message: `Update the Turborepo example '${entry.example}' only. Last commit ${entry.lastCommitAt}. Branch: agents/examples/${entry.example}. Open exactly one pull request for this example.`
    })
  )
);
return results;
```

When you are the child, your assignment is one example. Stay inside `examples/<name>`, and never fan out again.

## Version Updates

- Audit every versioned value that can go stale: dependencies, dev dependencies, peer dependencies, optional dependencies, `packageManager`, `engines.node`, Docker image tags, GitHub Action versions, and README command/version references.
- Resolve versions programmatically from their source of truth. Use npm registry metadata for npm packages and JavaScript package managers, Node release metadata for Node.js, and the relevant registry or release API for non-JavaScript toolchains.
- Update direct dependencies, package managers, and versioned toolchain values to exact latest stable versions from registry or release metadata. Never write the literal `latest` tag into manifests. Do not introduce loose ranges like `^` or `~` when updating examples.
- Major-version upgrades are expected. Apply them, then fix resulting breakage. Do not downgrade to the previous major to make validation easier.
- Treat `engines.node` as a versioned value to update to the latest stable Node release line unless a package manager literally cannot run on it. If the framework breaks on the latest Node, update or replace the framework/tooling usage until it works.
- Prefer the repository's current Turborepo version and documented task naming conventions.

## Best-Practice Migrations

- Treat upgrades as migrations, not mechanical package bumps. After each major upgrade, read the relevant release notes, migration guide, or current docs when needed and update the example to the new recommended shape.
- Remove deprecated configuration and APIs instead of carrying them forward. Do not keep legacy config files, flags, scripts, or compatibility adapters when the latest ecosystem has a clean replacement.
- Prefer idiomatic current defaults for the upgraded toolchain: flat config for modern ESLint, current TypeScript module settings where required, current framework config file names and options, current package-manager lockfile/install practices, and modern test/build config conventions.
- Keep examples teachable. A best-practice migration should leave templates simple, explicit, and copyable, not wrapped in clever abstraction to hide upgrade pain.
- Update docs and README commands to match the migrated behavior, not just the installed versions.
- When old dependencies exist only to support old patterns, remove or replace them with the current recommended package or built-in framework capability.
- Validate that the migrated configuration is actually used by the scripts. Passing validation with dead config is not sufficient.

## No-Downgrade Policy

- Never pin a direct dependency below registry `latest` because another package is not compatible yet.
- Never keep ESLint, TypeScript, React, Angular, Vite, Storybook, Express, Prisma, TypeORM, Nuxt, Vue, Expo, React Native, or any other direct dependency on an older major as a “latest compatible” compromise.
- If a latest package breaks due to another package, migrate away from the incompatible package, remove or replace the broken plugin/config, update code to new APIs, or restructure the example.
- For ESLint specifically, if `eslint-plugin-react` or legacy `.eslintrc` config blocks latest ESLint, migrate the example to flat config and either replace the incompatible plugin usage, drop nonessential React lint rules, or use framework/native lint coverage that works with latest ESLint. Do not pin ESLint 9 just because a plugin breaks on ESLint 10.
- External package patching is a last resort, but it is allowed if there is no viable migration or replacement and it is required to keep latest direct dependencies working. Keep patches small, documented in package-manager-native patch metadata, and generated programmatically where the package manager supports it.
- The only acceptable blocker is an unpublished package/version, unavailable registry/service, or missing credentials. Compatibility failures are not blockers; they are migration work.

## Lockfiles

- Never manually write lockfiles.
- After changing dependencies or `packageManager`, update lockfiles by running the example's declared package manager install command through `update_example_lockfile`.
- If package-manager install fails, fix the manifest or compatibility issue before validating tasks.

## Task Validation

- Use `audit_example_tasks` before validation to identify persistent and non-persistent tasks.
- Persistent tasks such as `dev`, `start`, `serve`, and `preview` are not pass/fail validation tasks.
- Non-persistent tasks such as `build`, `lint`, `test`, `check-types`, and framework-specific compile checks must pass after updates when present.
- Run the narrowest relevant verification commands for each changed example using `run_example_script`.
- If a version bump breaks an example, fix the breakage in the same pass instead of leaving the example half-updated.

## Pull Requests

- One example, one pull request. Never bundle several examples into a single pull request.
- Use the branch `agents/examples/<example>`. Re-running an example reuses that branch instead of creating a second one.
- Before opening a pull request, confirm the example was in `updateQueue` and not in `skipped`. An example already carrying an open pull request is done for this cycle; skip it and say so rather than opening a duplicate.
- Include the example name, the versions moved, and the validation results in the pull request body.

## Completion Contract

- For broad requests like "update our examples", completion means `find_stale_examples` has run, every queued example has been dispatched to its own run, and every skipped example has been reported with its reason. It does not mean this run touched the examples itself.
- For a single-example assignment, completion means that example's direct version pins have been moved to exact latest stable values, its lockfile has been regenerated with its declared package manager, best-practice migrations have been applied, all relevant non-persistent validation tasks have been attempted, and one pull request is open.
- Do not end a turn by saying there is remaining work on your example and that you will continue later. Continue in the same run.
- Do not present a checkpoint, progress report, blocker analysis, or plan as the final answer unless every remaining item is blocked by a true external blocker.
- A true external blocker is limited to unavailable registries/services, missing credentials, or an unpublished package/version. Build, lint, type, test, peer-dependency, framework, plugin, and migration failures are not blockers; they are work to fix.

## Recommended Tool Flow

In a fan-out run:

1. Use `find_stale_examples` to get the update queue and the examples to skip.
2. Use `Workflow` to dispatch one `tools.agent` call per queued example.

In a single-example run:

1. Use `inspect_example` to understand the example you own.
2. Use `audit_example_versions` with that example to find stale `package.json`, `packageManager`, and Node engine values.
3. Use `find_versioned_references` with that example to find versioned references outside manifests.
4. Use `audit_example_tasks` to identify validation scripts and persistent tasks.
5. Use `read_examples_file` before modifying existing files.
6. Use `write_examples_file` for non-lockfile example changes.
7. Use `update_example_lockfile` after dependency or package-manager changes.
8. Use `run_example_script` for each relevant non-persistent validation task.
9. Use `create_pull_request` once, on `agents/examples/<example>`.

## Reporting

- In a fan-out run, summarize the dispatched examples with their outcomes and pull requests, plus the skipped examples with their reasons.
- In a single-example run, summarize the example changed, exact versions selected, lockfile update commands run, validation results, and the pull request opened.
- If a check cannot run because dependencies or external services are unavailable, state that clearly and include the command that failed.
- Do not ask the user to choose safe vs full updates. The default is full latest exact pins for every example you are given.
- Do not report “latest-compatible” fallbacks as completion. Completion requires exact latest direct pins or a true external availability blocker.
- Report only after the completion contract is satisfied. Avoid interim status updates unless a tool or channel requires visible progress.
