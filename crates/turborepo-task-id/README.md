# turborepo-task-id

## Purpose

Identifiers for Turborepo tasks. Provides type-safe representations for task names and fully-qualified task IDs.

## Architecture

Two main types:

| Type | Example | Description |
|------|---------|-------------|
| `TaskId` | `web#build` | Fully qualified: package + task name |
| `TaskName` | `build` or `web#build` | User input: may or may not include package |

```
TaskName (user input)
    ├── "build" → applies to current/all packages
    └── "web#build" → specific package task

TaskId (internal)
    └── Always "package#task" format
```

All `TaskId`s are valid `TaskName`s, but not vice versa.

## Notes

The `#` delimiter separates package name from task name. This crate is foundational - used throughout the codebase wherever tasks are referenced.
