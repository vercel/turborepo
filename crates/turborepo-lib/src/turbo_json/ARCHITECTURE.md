# Turborepo Architecture: Configuration and Task Definition Loading

This document explains the process of loading `turbo.json` files and creating task definitions in Turborepo.
The process involves several phases that transform raw configuration files into an executable task graph.
The loading and resolving of task definitions is driven during task graph construction.

## Overview

The configuration and task loading process follows this high-level flow:

1. **Configuration Resolution**: Collect and merge configuration from multiple sources
2. **TurboJson Loading**: Resolve `turbo.json` files, these are usually files on disk, but can be synthesized
3. **Task Definition Resolution**: Convert raw task definitions into validated structures. `extends` is handled in this step
4. **Task Graph Construction**: Build the executable task graph from resolved definitions

## Phase 1: Configuration Resolution

### Sources and Priority

Configuration is collected from multiple sources with the following priority (highest to lowest):

1. Command line arguments
2. Environment variables
3. Override environment variables
4. Local configuration (`.turbo/config.json`)
5. Global authentication (`~/.turbo/auth.json`)
6. Global configuration (`~/.turbo/config.json`)
7. Turbo.json configuration

### Key Components

- **`TurborepoConfigBuilder`** (`crates/turborepo-lib/src/config/mod.rs`): Orchestrates the configuration loading process
- **`TurboJsonReader`** (`crates/turborepo-lib/src/config/turbo_json.rs`): Extracts configuration options from the root `turbo.json` file
- **`ConfigurationOptions`**: The final merged configuration structure

## Phase 2: TurboJson Loading

### Key Components

- **`TurboJsonLoader`** (`crates/turborepo-lib/src/turbo_json/loader.rs`): Loads and resolves turbo.json files
- **`RawTurboJson`**: Raw deserialized structure from JSON files
- **`TurboJson`**: Resolved and validated structure, all DSL magic strings have been handled

### Process

1. **File Discovery**: Locate `turbo.json` or `turbo.jsonc` files
2. **Parsing**: Deserialize JSON into `RawTurboJson` structures
3. **Validation**: Convert to `TurboJson` with validation rules
4. **Workspace Resolution**: Apply workspace-specific overrides

## Phase 3: Task Definition Resolution

### Key Components

- **`RawTaskDefinition`**: Raw task configuration from JSON
- **`TaskDefinition`**: Validated and processed task configuration
- **`TaskId`** and **`TaskName`** (from `turborepo-task-id` crate): Task identification types

### Transformation Process

Raw task definitions undergo several transformations:

1. **Path Resolution**: Convert relative paths and handle `$TURBO_ROOT$` tokens
2. **Dependency Parsing**: Parse `dependsOn` into topological and task dependencies
3. **Environment Variable Collection**: Extract `env` and `passThroughEnv` variables
4. **Output Processing**: Handle inclusion/exclusion patterns in outputs
5. **Inheritance**: Handle merging multiple `RawTaskDefinition`s into a single usable task definition
6. **Validation**: Ensure configuration consistency (e.g., interactive tasks can't be cached)
