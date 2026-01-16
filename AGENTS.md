# AGENTS.md

Instructions for AI agents working on this repository.

## Architecture

See [ARCHITECTURE.md](./crates/turborepo/ARCHITECTURE.md) for an overview of the `turbo run` command architecture.

## Keeping Documentation Up to Date

When making changes to the codebase, check if the following docs need updates:

- **[ARCHITECTURE.md](./crates/turborepo/ARCHITECTURE.md)** - Update when changing core `turbo run` components:
  - Run builder, package graph, task graph/engine
  - Task visitor, caching system, task hashing
  - Run tracking and summary generation
  - Any files in `crates/turborepo-lib/src/run/`, `crates/turborepo-lib/src/engine/`, `crates/turborepo-lib/src/task_graph/`, or `crates/turborepo-cache/`

- **[CONTRIBUTING.md](./CONTRIBUTING.md)** - Update when changing:
  - Build process or development setup
  - Testing procedures or requirements
  - Project structure or tooling

- **This file (AGENTS.md)** - Update when changing:
  - PR requirements or CI workflows
  - Repository conventions or policies

## Pull Request Guidelines

### PR Title Format

PR titles must follow [Conventional Commits](https://www.conventionalcommits.org/). See [`.github/workflows/lint-pr-title.yml`](./.github/workflows/lint-pr-title.yml) for the enforced constraints.

Format: `<type>: <Description>`

Key rules:

- Description must start with an uppercase letter
- Scopes are not allowed

Examples:

```
feat: Add new cache configuration option
fix: Resolve race condition in task scheduling
docs: Update installation instructions
```
