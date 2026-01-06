# AGENTS.md

Instructions for AI agents working on this repository.

## Architecture

See [ARCHITECTURE.md](./ARCHITECTURE.md) for an overview of the `turbo run` command architecture.

## Pull Request Guidelines

### PR Title Format

PR titles must follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
<type>(<scope>): <description>
```

- **type**: Required. Must be one of:
  - `fix` - Bug fixes
  - `feat` - New features
  - `chore` - Maintenance tasks
  - `ci` - CI/CD changes
  - `docs` - Documentation changes
  - `refactor` - Code refactoring
  - `perf` - Performance improvements
  - `test` - Test changes
  - `style` - Code style changes
- **scope**: Optional, but `examples` and `example` are not allowed as scopes
- **description**: A short summary of the change, must start with an uppercase letter

### Examples

```
feat(cli): Add new cache configuration option
fix(turbo): Resolve race condition in task scheduling
docs: Update installation instructions
chore(deps): Bump typescript to v5.3
refactor(core): Simplify task graph construction
```

### Rules

1. Use one of the allowed conventional commit types listed above
2. The subject (description) must start with an uppercase letter
3. Scope is optional but cannot be `examples` or `example`
4. Keep the title concise (ideally under 72 characters)
