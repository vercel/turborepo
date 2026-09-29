---
name: turborepo
description: Configure and troubleshoot Turborepo repositories. Use when working with turbo.json, task pipelines, caching, Remote Cache, the turbo CLI, filtering, environment variables, package boundaries, monorepo structure, or CI workflows.
---

# Turborepo

The complete Turborepo documentation ships inside the installed `turbo` package. Do not rely on this skill for framework guidance. Always read the bundled docs, which match the installed version exactly.

Start with:

```text
node_modules/turbo/docs/README.md
```

Use that task index to choose the smallest relevant documentation page. Read it before changing Turborepo configuration, package scripts, or CI workflows.

If the package manager uses a non-flat `node_modules` layout or a workspace link, resolve the package location first:

```sh
node -p "require.resolve('turbo/package.json')"
```

Then read `docs/README.md` relative to the resolved package directory.

If `turbo` is not installed, inspect the repository's package manager and existing version constraints before adding it. After installation, use the bundled docs rather than guidance for a different release.
