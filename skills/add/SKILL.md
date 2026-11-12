---
name: add
description: Add Turborepo conservatively to an existing package-manager repository. Use when a repository needs minimal selected-task Turborepo configuration, including when another setup flow needs Turborepo before enabling its own feature.
---

# Add Turborepo

Add Turborepo conservatively to an existing package-manager repository, preserving package scripts and current behavior. This skill adds Turborepo as an additional task runner path for selected tasks; it does not convert every repository workflow to Turborepo.

Use this skill for repositories with a root `package.json` and selected package scripts. For workspace repositories, configure selected workspace package tasks. For single-package repositories, configure only selected root tasks.

## Required documentation

Read the current official guidance before changing files:

- [Add to an existing repository](https://turborepo.dev/docs/getting-started/add-to-existing-repository)
- [Configuring tasks](https://turborepo.dev/docs/crafting-your-repository/configuring-tasks)
- [Configuration reference](https://turborepo.dev/docs/reference/configuration)
- [Package configurations](https://turborepo.dev/docs/reference/package-configurations) when package-level overrides may be needed

Treat these pages as the source of truth for current configuration and package-manager commands. If required documentation cannot be verified, stop rather than relying on remembered syntax.

`turbo setup` is managed-toolchain/dependency setup for repositories that already have a root `turbo.json` or `turbo.jsonc` and supported toolchain declarations. It is not a replacement for adding Turborepo. Do not introduce `turbo.lock`, toolchain declarations, or `setup` configuration solely for this flow unless the user explicitly requests managed toolchains.

## Applicability

Require all of the following:

1. A clean local Git checkout. Never discard unrelated work.
2. A root `package.json` and one lockfile for npm, pnpm, Yarn, or Bun.
3. Either a single root package or an existing package-manager workspace with packages.
4. A package or workspace layout that can remain unchanged.
5. Selected package scripts whose current orchestration and environment requirements can be determined from repository configuration and CI workflows.

Stop instead of changing files when:

- the repository has no root `package.json` or no selected package scripts to configure;
- adding Turborepo requires changing package managers, moving packages, or restructuring applications;
- multiple conflicting lockfiles make package-manager ownership ambiguous;
- required task ordering, generated outputs, or environment dependencies cannot be inferred safely.

Explain the failed condition and leave the repository unchanged.

## Workflow

### 1. Analyze without changing files

Inspect:

- the root package manifest, workspace definition when present, lockfile, and package manifests;
- root scripts that run selected root tasks or orchestrate workspace commands;
- scripts shared by multiple packages when workspaces are present;
- package dependency relationships when workspaces are present;
- GitHub Actions and other CI configuration for task ordering, filters, outputs, services, and environment variables;
- existing build artifacts and ignored generated directories.

Summarize:

- detected package manager and package or workspace layout;
- candidate tasks and package coverage;
- observed dependencies, outputs, and environment requirements;
- behavior that cannot be inferred.

For standalone use, a direct user request to add Turborepo is enough to proceed within the safe boundaries below; ask for confirmation only when the request is ambiguous or would require broadening the change. When launched by another setup skill, that setup request is enough to proceed after explaining that Turborepo is required, but only when the change remains inside the same safe boundaries. Stop instead of broadening the change silently.

### 2. Establish the boundary

Configure only tasks whose behavior is clear. Prefer focused lint, type-check, formatter-check, unit-test, and build tasks already run by the root package or across workspaces.

Do not:

- rename or move packages;
- change package managers or workspace definitions;
- rewrite package-level scripts;
- add remote caching;
- add product-specific CI tags or enable another product's CI feature;
- configure deployment, end-to-end, integration, or environment-provisioning tasks unless the user explicitly requests them.

Preserve existing root scripts by default. When retaining an existing root orchestrator, name its Turbo alternative `<task>:turbo`. Replace an existing orchestration script only when the Turbo command is demonstrably equivalent.

### 3. Add Turborepo

When launched by another setup skill for a same-PR setup flow, make the change in that setup's current branch or worktree. For standalone use or separate review, create a branch from the latest remote default branch. If there is no remote default branch, stop unless the user explicitly asked for a local-only test.

Install `turbo` as a root development dependency with the repository's existing package manager. Preserve its dependency style and regenerate the lockfile through the package manager; never edit the lockfile manually.

Ensure the root `package.json` declares the package manager according to current Turborepo guidance. Preserve an existing valid `packageManager`; otherwise add `devEngines.packageManager` using the detected package manager name and exact version from the lockfile or package-manager command. Do not guess a version.

Create a root `turbo.json` or `turbo.jsonc` using the current documented schema. Configure only the selected tasks:

- encode task dependencies only when repository evidence establishes them;
- declare generated outputs when they can be identified confidently;
- include shared root files used by package tasks in task `inputs`; when adding custom `inputs`, include `"$TURBO_DEFAULT$"` so package source-control inputs and ignore behavior are preserved, and use `"$TURBO_ROOT$/<path>"` for root files outside a package;
- disable caching for side-effecting tasks or tasks whose meaningful outputs cannot be captured safely;
- declare environment variables according to current Turborepo guidance when repository evidence establishes them;
- leave uncertain tasks out of the initial Turborepo configuration.

Do not add speculative `dependsOn`, `inputs`, `outputs`, or environment configuration.

Preserve existing ignore rules. Ensure the package manager's dependency directory and Turborepo's local `.turbo` directory are ignored without replacing the repository's `.gitignore`.

### 4. Validate behavior

Install dependencies using the lockfile and repository's normal immutable or frozen mode when available. Do not use `turbo setup` for validation unless the repository already used managed toolchains before this change.

First inspect the Turbo dry-run output and confirm:

- only intended packages and tasks are selected;
- task dependencies match observed ordering;
- no package with a matching script is unexpectedly omitted;
- no package without the script is forced to run it;
- changing a shared root input invalidates the affected task cache. For tasks with shared root inputs, run once to populate cache, run again to confirm a cache hit, change one root input, confirm the affected task misses cache, then revert the change.

Then run the existing command and its Turbo equivalent for each selected low-risk task. Compare package coverage, ordering where material, exit status, and expected outputs. Do not claim equivalence from a successful Turbo command alone.

If validation reveals uncertain behavior, narrow the configuration or stop. Never conceal a behavioral difference by deleting the old command.

### 5. Return or publish the change

When launched by another setup skill for a same-PR setup flow, do not open a separate pull request. Return the diff and validation results so the caller can include the Turborepo addition in its pull request.

When launched standalone, or when the user asks for separate review, review the complete diff, commit using the repository's commit style, push the branch, and open a draft pull request. Do not include another product's CI enablement or task tags in the Turborepo-only pull request. Do not merge it unless the user explicitly asks.

Whether the Turborepo addition is included in another pull request or published separately, the summary must include:

- why Turborepo is being added;
- the tasks configured and package coverage;
- preserved scripts and any root script intentionally replaced;
- inferred dependencies, outputs, inputs, and environment configuration;
- validation comparing existing and Turbo commands;
- deferred or unsupported tasks.

## Handoff

Return the selected tasks, package coverage, validation results, deferred tasks, and exact commit or diff. When a separate pull request was created, also return the pull-request URL, head branch, base branch, and exact head commit.

Another setup skill may include this change in its own pull request only when the change is limited to selected tasks and validation proves the Turbo commands preserve existing behavior. If the caller or user needs separate review, use the Turborepo-only pull request path and tell the user to rerun the caller's setup after the Turborepo pull request merges.
