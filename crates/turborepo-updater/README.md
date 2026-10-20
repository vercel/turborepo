# turbo-updater

## Purpose

Notifies users when a new version of `turbo` is available. Checks the Turborepo API for updates and displays a styled terminal message with upgrade instructions.

## Architecture

```
display_update_check()
        │
        ▼
┌───────────────────┐
│ should_skip_      │──▶ Skip if: config disabled, NO_UPDATE_NOTIFIER,
│ notification()    │             CI env var, or non-TTY
└───────────────────┘
        │
        ▼
┌───────────────────┐
│ check_for_updates │──▶ Uses update-informer with custom NPMRegistry
└───────────────────┘    Fetches from turborepo.dev/api/binaries/version
        │
        ▼
┌───────────────────┐
│ ui::message()     │──▶ Renders responsive box based on terminal width
└───────────────────┘
```

**Key components:**

- `NPMRegistry` - Custom `update-informer` registry implementation that queries Turborepo's version API
- `VersionTag` - Differentiates between `latest` and `canary` release channels
- `ui` module - Handles responsive terminal rendering with box drawing

## Notes

- **Disabled in CI**: Skips notification when `CI` env var is set
- **User opt-out**: Respects `NO_UPDATE_NOTIFIER` env var
- **TTY only**: Won't display if stdout is not a terminal
- **Caching**: `update-informer` caches results for 24 hours (configurable via `interval`)
- **Timeout**: Version check has 800ms timeout to avoid blocking CLI startup
- **Channel-aware**: Canary versions only see canary updates; stable sees stable
