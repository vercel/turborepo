# Turborepo Architecture: Configuration and Task Definition Loading

This document explains the process of loading `turbo.json` files and creating task definitions in Turborepo.
The process involves several phases that transform raw configuration files into an executable task graph.
The loading and resolving of task definitions is driven during task graph construction.

## Overview

The configuration and task loading process follows this high-level flow:

1. **Configuration Resolution**: Collect and merge configuration from multiple sources
2. **TurboJson Loading**: Parse `turbo.json` files into raw structures
3. **Task Processing**: Convert raw definitions to processed intermediate representation with DSL token handling
4. **Task Definition Resolution**: Transform processed definitions into final validated structures
5. **Task Graph Construction**: Build the executable task graph from resolved definitions

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

### Schema Differentiation

Turborepo uses two different schemas for `turbo.json` files depending on their location:

- **Root `turbo.json`** (`RawRootTurboJson`): Located at the repository root

  - Can define global configuration options (`globalEnv`, `globalDependencies`, `globalPassThroughEnv`)
  - Can set repository-wide settings (`remoteCache`, `ui`, `daemon`, `envMode`, etc.)
  - Can define `futureFlags` for experimental features
  - Cannot use `extends` field

- **Package `turbo.json`** (`RawPackageTurboJson`): Located in workspace packages
  - Limited to task definitions and workspace-specific configuration
  - Must use `extends: ["//"]` to inherit from root configuration
  - Can define workspace-specific `tags` and `boundaries`
  - Cannot define global settings or `futureFlags`

### Key Components

- **`TurboJsonLoader`** (`crates/turborepo-lib/src/turbo_json/loader.rs`): Loads and resolves turbo.json files
- **`RawRootTurboJson`** (`crates/turborepo-lib/src/turbo_json/raw.rs`): Root turbo.json schema
- **`RawPackageTurboJson`** (`crates/turborepo-lib/src/turbo_json/raw.rs`): Package turbo.json schema
- **`RawTurboJson`** (`crates/turborepo-lib/src/turbo_json/raw.rs`): Unified raw structure that can represent either type
- **`TurboJson`**: Validated structure containing raw task definitions

### Process

1. **File Discovery**: Locate `turbo.json` or `turbo.jsonc` files
2. **Schema Detection**: Determine if file is at repository root or in a package
3. **Parsing**: Deserialize JSON into appropriate schema (`RawRootTurboJson` or `RawPackageTurboJson`)
4. **Unification**: Convert to unified `RawTurboJson` structure
5. **Basic Validation**: Convert to `TurboJson` with structural validation
6. **Workspace Resolution**: Apply workspace-specific overrides

## Phase 3: Processed Task Definition (Intermediate Representation)

### Key Components

- **`ProcessedTaskDefinition`** (`crates/turborepo-lib/src/turbo_json/processed.rs`): Intermediate representation with DSL token processing
- **`ProcessedGlob`**: Parsed glob patterns with separated components (base pattern, negation flag, turbo_root flag)
- **`ProcessedInputs`/`ProcessedOutputs`**: Collections of processed globs with resolution methods

### Processing Steps

1. **DSL Token Detection**: Identify and separate `$TURBO_ROOT$` and `!` prefixes from glob patterns
2. **Early Validation**: Single-field validations at parse time with span information:
   - Absolute paths in inputs/outputs
   - Invalid `$TURBO_ROOT$` usage
   - Environment variable prefixes (`$` not allowed)
   - Dependency prefixes (`$` not allowed in `dependsOn`)
   - Topological references (`^` not allowed in `with`)
3. **Prefix Stripping**: Store clean glob patterns without DSL prefixes
4. **Component Separation**: Track negation and turbo_root requirements as separate boolean flags

## Phase 4: Task Definition Resolution

### Key Components

- **`RawTaskDefinition`**: Raw task configuration from JSON
- **`ProcessedTaskDefinition`**: Intermediate representation with parsed DSL tokens
- **`TaskDefinition`**: Final validated and resolved task configuration
- **`TaskId`** and **`TaskName`** (from `turborepo-task-id` crate): Task identification types

### Transformation Process

The resolution now follows a three-stage pipeline:

1. **Raw → Processed** (`ProcessedTaskDefinition::from_raw`):

   - Parse glob patterns and extract DSL tokens
   - Validate single-field constraints with span information:
     - Absolute paths in inputs/outputs (`ProcessedGlob::from_spanned_*`)
     - Invalid environment variable prefixes (`ProcessedEnv::new`, `ProcessedPassThroughEnv::new`)
     - Invalid dependency syntax (`ProcessedDependsOn::new`)
     - Invalid sibling task references (`ProcessedWith::new`)
   - Strip prefixes and store components separately

2. **Processed → Resolved** (`TaskDefinition::from_processed`):

   - Apply `$TURBO_ROOT$` token replacement using `resolve()` methods
   - Parse `dependsOn` into topological and task dependencies
   - Transform environment variables into sorted lists
   - Transform outputs into inclusion/exclusion patterns
   - Validate multi-field constraints:
     - Interactive tasks cannot be cached (requires `cache` and `interactive` fields)
     - Interruptible tasks must be persistent (requires `interruptible` and `persistent` fields)

3. **Inheritance**: The `extend` module handles merging multiple `ProcessedTaskDefinition`s before final resolution
