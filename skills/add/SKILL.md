---
name: add
description: Add Turborepo to an existing repository in any toolchain Turborepo supports. Use when asked to add, introduce, or set up Turborepo or turbo in a repository that does not use it yet.
---

# Add Turborepo

Add Turborepo to an existing repository for selected tasks, without changing what existing commands do. In workspace repositories, configure workspace package tasks. In single-package repositories, configure root tasks.

## Documentation

Read these before changing files, and treat them as the source of truth for configuration syntax and toolchain prerequisites:

- [Add to an existing repository](https://turborepo.dev/docs/getting-started/add-to-existing-repository)
- [Configuring tasks](https://turborepo.dev/docs/crafting-your-repository/configuring-tasks)
- [Configuration reference](https://turborepo.dev/docs/reference/configuration)
- [Package configurations](https://turborepo.dev/docs/reference/package-configurations), if package-level overrides are needed
- The guide for each toolchain in use, such as [Rust](https://turborepo.dev/docs/guides/tools/rust), [Python](https://turborepo.dev/docs/guides/tools/python), or [Go](https://turborepo.dev/docs/guides/tools/go)

Once `turbo` is installed, prefer its bundled `docs/README.md`, which matches the installed version. If you cannot verify the docs, stop instead of relying on remembered syntax.

## When to stop

Stop and explain why, leaving the repository unchanged, if:

- the Git checkout has uncommitted changes;
- Turborepo cannot discover packages from the repository's existing manifests;
- adding Turborepo would require a new toolchain, package manager, or manifest type, or would require moving packages;
- one toolchain has conflicting lockfiles;
- you cannot tell from the repository and CI how any selected task is ordered, what it outputs, or what environment it needs.

## Workflow

### 1. Analyze

Read the manifests, workspace definition, lockfiles, root scripts or task-runner targets, package dependency relationships, and CI configuration. Summarize the toolchains, layout, candidate tasks, observed dependencies, outputs, environment variables, and anything you could not infer.

A direct request to add Turborepo is enough to proceed. Ask first only if the request is ambiguous or would widen the scope below.

### 2. Choose tasks

Configure only tasks whose behavior is clear, preferring lint, type-check, format-check, unit-test, and build tasks that the repository already runs.

Do not:

- rename or move packages, or change toolchains, package managers, or workspace membership;
- rewrite package-level task commands;
- add Remote Caching;
- configure deploy, end-to-end, integration, or environment-provisioning tasks unless asked.

Keep existing root entry points. Give the `turbo` version of a root orchestrator its own name following the repository's convention, such as `<task>:turbo`. Replace the original only if the `turbo` command is proven equivalent.

### 3. Add Turborepo

Install `turbo` as the toolchain's guide describes. If the repository pins its tool versions, pin `turbo` to an exact version the same way. Regenerate lockfiles with the tool that owns them. Never edit a lockfile by hand.

Add any prerequisites from the toolchain guide that are missing, such as a workspace name or toolchain declaration. Take versions from the lockfile or the toolchain's own command instead of guessing.

Create a root `turbo.json` for the selected tasks only:

- Enable only the toolchain support those tasks need, including any required `futureFlags`. Tell the user if that support is experimental.
- Add `dependsOn`, `inputs`, `outputs`, and environment variables only when the repository shows they are needed.
- If you add custom `inputs`, include `"$TURBO_DEFAULT$"`. Reference root files outside a package with `"$TURBO_ROOT$/<path>"`.
- Disable caching for side-effecting tasks and for tasks whose outputs cannot be captured.

Toolchain support can register built-in tasks you did not select. Report them as deferred, not validated.

Add the dependency directory and `.turbo` to `.gitignore` if they are missing. Do not change any other ignore rules.

### 4. Validate

Install dependencies in frozen or immutable mode. Then check a `turbo run <task> --dry` output for each task:

- Only the intended packages and tasks are selected, and no package that defines a task is missing.
- Task ordering matches what the repository does today.
- For tasks with shared root inputs: run once, confirm a cache hit on the second run, change one root input, confirm a cache miss, then revert the change.

Then run each existing command and its `turbo` equivalent, and compare package coverage, ordering, exit status, and outputs. A passing `turbo` run alone does not prove equivalence. If behavior differs, narrow the configuration or stop. Never delete the old command to hide a difference.

### 5. Report

Summarize:

- the configured tasks and package coverage;
- preserved commands and any replaced root entry points;
- inferred dependencies, inputs, outputs, and environment variables;
- validation results comparing existing and `turbo` commands;
- deferred or unsupported tasks.

Commit, push, or open a pull request only if the user asks. Open it as a draft.

## When another skill calls this one

Work in the caller's branch, do not open a pull request, and return the diff, validation results, and deferred tasks to the caller. If the change would need to go beyond the selected tasks, stop instead of widening the scope.
