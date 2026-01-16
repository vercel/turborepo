//! turbo-schema-gen: Generate JSON Schema and TypeScript types from Rust types
//!
//! This binary generates both `schema.json` and TypeScript type definitions
//! from the Rust source of truth in `turborepo-turbo-json`.
//!
//! ## Usage
//!
//! ```bash
//! # Generate JSON Schema to stdout
//! turbo-schema-gen schema
//!
//! # Generate TypeScript types to stdout  
//! turbo-schema-gen typescript
//!
//! # Write to files
//! turbo-schema-gen schema -o schema.json
//! turbo-schema-gen typescript -o types.ts
//! ```

use std::{fs, io::Write, path::PathBuf};

use clap::{Parser, Subcommand};
use schemars::{schema::RootSchema, schema_for};
use ts_rs::TS;
use turborepo_turbo_json::RawTurboJson;
use turborepo_types::{EnvMode, OutputLogsMode, UIMode};

/// Generate JSON Schema and TypeScript types for turbo.json
#[derive(Parser)]
#[command(name = "turbo-schema-gen")]
#[command(about = "Generate JSON Schema and TypeScript types from Rust types")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate JSON Schema for turbo.json
    Schema {
        /// Output file path (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Pretty print the JSON (default: true)
        #[arg(long, default_value = "true")]
        pretty: bool,
    },

    /// Generate TypeScript type definitions
    Typescript {
        /// Output file path (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Verify generated files match current Rust types
    Verify {
        /// Path to existing schema.json
        #[arg(long)]
        schema: Option<PathBuf>,

        /// Path to existing TypeScript types
        #[arg(long)]
        typescript: Option<PathBuf>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Schema { output, pretty } => {
            let schema = generate_schema();
            let json = if pretty {
                serde_json::to_string_pretty(&schema)?
            } else {
                serde_json::to_string(&schema)?
            };

            write_output(&json, output.as_ref())?;
        }

        Commands::Typescript { output } => {
            let typescript = generate_typescript();
            write_output(&typescript, output.as_ref())?;
        }

        Commands::Verify { schema, typescript } => {
            let mut success = true;

            if let Some(schema_path) = schema
                && !verify_schema(&schema_path)?
            {
                success = false;
            }

            if let Some(ts_path) = typescript
                && !verify_typescript(&ts_path)?
            {
                success = false;
            }

            if !success {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// Generate the JSON Schema for turbo.json (both root and workspace schemas)
fn generate_schema() -> RootSchema {
    // Generate schema for RawTurboJson which is the complete turbo.json structure
    // This includes both root-level and workspace-level configurations
    schema_for!(RawTurboJson)
}

/// Generate TypeScript type definitions
fn generate_typescript() -> String {
    let mut output = String::new();

    // Enum types first (matching original order)
    add_type_decl::<OutputLogsMode>(&mut output, "");
    add_type_decl::<EnvMode>(&mut output, "");
    add_type_decl::<UIMode>(&mut output, "");

    // Simple type aliases for compatibility
    output.push_str("/**\n");
    output.push_str(
        " * This is a relative Unix-style path (e.g. `./src/index.ts` or `src/index.ts`).  \
         Absolute paths (e.g. `/tmp/foo`) are not valid.\n",
    );
    output.push_str(" */\n");
    output.push_str("export type RelativeUnixPath = string;\n");
    output.push_str("export type EnvWildcard = string;\n\n");

    // Generate interface types (in original file order)
    output.push_str(&generate_base_schema_interface());
    output.push_str(&generate_workspace_schema_interface());
    output.push_str(&generate_root_schema_interface());
    output.push_str(&generate_pipeline_interface());
    output.push_str(&generate_remote_cache_interface());
    output.push_str(&generate_permissions_interface());
    output.push_str(&generate_tag_rules_interface());
    output.push_str("export type BoundariesRulesMap = Record<string, TagRules>;\n\n");
    output.push_str(&generate_boundaries_config_interface());
    output.push_str(&generate_root_boundaries_config_interface());

    // Type guards at the end
    output.push_str("export const isRootSchemaV2 = (schema: Schema): schema is RootSchema =>\n");
    output.push_str("  !(\"extends\" in schema);\n\n");
    output.push_str("export const isWorkspaceSchemaV2 = (\n");
    output.push_str("  schema: Schema\n");
    output.push_str("): schema is WorkspaceSchema => !isRootSchemaV2(schema);\n\n");
    output.push_str("export type Schema = RootSchema | WorkspaceSchema;\n");

    output
}

/// Generate the Permissions interface
fn generate_permissions_interface() -> String {
    r#"export interface Permissions {
  /**
   * Lists which tags are allowed. Any tag not included will be banned
   * If omitted, all tags are permitted
   */
  allow?: Array<string>;
  /**
   * Lists which tags are banned.
   */
  deny?: Array<string>;
}

"#
    .to_string()
}

/// Generate the TagRules interface (Rule in Rust)
fn generate_tag_rules_interface() -> String {
    r#"interface TagRules {
  /**
   * Rules for a tag's dependencies. Restricts which packages a tag can import
   */
  dependencies?: Permissions;
  /**
   * Rules for a tag's dependents. Restricts which packages can import this tag.
   */
  dependents?: Permissions;
}

"#
    .to_string()
}

/// Generate the BoundariesConfig interface (for workspace configs)
fn generate_boundaries_config_interface() -> String {
    r#"export interface BoundariesConfig {
  /**
   * Declares any implicit dependencies, i.e. any dependency not declared in a package.json.
   * These can include dependencies automatically injected by a framework or a testing library.
   */
  implicitDependencies?: Array<string>;
}

"#
    .to_string()
}

/// Generate the RootBoundariesConfig interface (for root config)
fn generate_root_boundaries_config_interface() -> String {
    r#"export interface RootBoundariesConfig extends BoundariesConfig {
  /**
   * The boundaries rules for tags. Restricts which packages
   * can import a tag and which packages a tag can import
   */
  tags?: BoundariesRulesMap;
}

"#
    .to_string()
}

/// Generate the RemoteCache interface
fn generate_remote_cache_interface() -> String {
    r#"export interface RemoteCache {
  /**
   * Indicates if signature verification is enabled for requests to the remote cache. When
   * `true`, Turborepo will sign every uploaded artifact using the value of the environment
   * variable `TURBO_REMOTE_CACHE_SIGNATURE_KEY`. Turborepo will reject any downloaded artifacts
   * that have an invalid signature or are missing a signature.
   *
   * @defaultValue `false`
   */
  signature?: boolean;

  /**
   * Indicates if the remote cache is enabled. When `false`, Turborepo will disable
   * all remote cache operations, even if the repo has a valid token. If true, remote caching
   * is enabled, but still requires the user to login and link their repo to a remote cache.
   * Documentation: https://turborepo.dev/docs/core-concepts/remote-caching
   *
   * @defaultValue `true`
   */
  enabled?: boolean;

  /**
   * When enabled, any HTTP request will be preceded by an OPTIONS request to
   * determine if the request is supported by the endpoint.
   *
   * Documentation: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS#preflighted_requests
   *
   * @defaultValue `false`
   */
  preflight?: boolean;
  /**
   * Set endpoint for API calls to the remote cache.
   * Documentation: https://turborepo.dev/docs/core-concepts/remote-caching#self-hosting
   *
   * @defaultValue `"https://vercel.com/api"`
   */
  apiUrl?: string;
  /**
   * Set endpoint for requesting tokens during `turbo login`.
   * Documentation: https://turborepo.dev/docs/core-concepts/remote-caching#self-hosting
   *
   * @defaultValue `"https://vercel.com"`
   */
  loginUrl?: string;
  /**
   * Sets a timeout for remote cache operations. Value is given in seconds and
   * only whole values are accepted. If `0` is passed, then there is no timeout
   * for any cache operations.
   *
   * @defaultValue `30`
   */
  timeout?: number;
  /**
   * Sets a timeout for remote cache uploads. Value is given in seconds and
   * only whole values are accepted. If `0` is passed, then there is no timeout
   * for any remote cache uploads.
   *
   * @defaultValue `60`
   */
  uploadTimeout?: number;

  /**
   * The ID of the Remote Cache team. Value will be passed as `teamId` in the
   * querystring for all Remote Cache HTTP calls. Must start with `team_` or it will
   * not be used.
   */
  teamId?: string;

  /**
   * The slug of the Remote Cache team. Value will be passed as `slug` in the
   * querystring for all Remote Cache HTTP calls.
   */
  teamSlug?: string;
}

"#
    .to_string()
}

/// Generate the Pipeline interface (task definition)
fn generate_pipeline_interface() -> String {
    r#"export interface Pipeline {
  /**
   * A human-readable description of what this task does.
   *
   * This field is for documentation purposes only and does not affect
   * task execution or caching behavior.
   */
  description?: string;

  /**
   * The list of tasks that this task depends on.
   *
   * Prefixing an item in dependsOn with a ^ prefix tells turbo that this task depends
   * on the package's topological dependencies completing the task first.
   * (e.g. "A package's build tasks should only run once all of its workspace dependencies
   * have completed their own build commands.")
   *
   * Items in dependsOn without a ^ prefix express the relationships between tasks within the
   * same package (e.g. "A package's test and lint commands depend on its own build being
   * completed first.")
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#dependson
   *
   * @defaultValue `[]`
   */
  dependsOn?: Array<string>;

  /**
   * A list of environment variables that this task depends on.
   *
   * Note: If you are migrating from a turbo version 1.5 or below,
   * you may be used to prefixing your variables with a $.
   * You no longer need to use the $ prefix.
   * (e.g. $GITHUB_TOKEN → GITHUB_TOKEN)
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#env
   *
   * @defaultValue `[]`
   */
  env?: Array<EnvWildcard>;

  /**
   * An allowlist of environment variables that should be made available in this
   * task's environment, but should not contribute to the task's cache key,
   * e.g. `AWS_SECRET_KEY`.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#passthroughenv
   *
   * @defaultValue `null`
   */
  passThroughEnv?: null | Array<EnvWildcard>;

  /**
   * The set of glob patterns indicating a task's cacheable filesystem outputs.
   *
   * Turborepo captures task logs for all tasks. This enables us to cache tasks whose runs
   * produce no artifacts other than logs (such as linters). Logs are always treated as a
   * cacheable artifact and never need to be specified.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#outputs
   *
   * @defaultValue `[]`
   */
  outputs?: Array<string>;

  /**
   * Whether or not to cache the outputs of the task.
   *
   * Setting cache to false is useful for long-running "watch" or development mode tasks.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#cache
   *
   * @defaultValue `true`
   */
  cache?: boolean;

  /**
   * The set of glob patterns to consider as inputs to this task.
   *
   * Changes to files covered by these globs will cause a cache miss and
   * the task will be rerun.
   *
   * If a file has been changed that is **not** included in the set of globs,
   * it will not cause a cache miss.
   *
   * If omitted or empty, all files in the package are considered as inputs.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#inputs
   *
   * @defaultValue `[]`
   */
  inputs?: Array<string>;

  /**
   * Output mode for the task.
   *
   * "full": Displays all output
   *
   * "hash-only": Show only the hashes of the tasks
   *
   * "new-only": Only show output from cache misses
   *
   * "errors-only": Only show output from task failures
   *
   * "none": Hides all task output
   *
   * Documentation: https://turborepo.dev/docs/reference/run#--output-logs-option
   *
   * @defaultValue `"full"`
   */
  outputLogs?: OutputLogs;

  /**
   * Indicates whether the task exits or not. Setting `persistent` to `true` tells
   * turbo that this is a long-running task and will ensure that other tasks
   * cannot depend on it.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#persistent
   *
   * @defaultValue `false`
   */
  persistent?: boolean;

  /**
   * Mark a task as interactive allowing it to receive input from stdin.
   * Interactive tasks must be marked with "cache": false as the input
   * they receive from stdin can change the outcome of the task.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#interactive
   *
   * @defaultValue `false`
   */
  interactive?: boolean;

  /**
   * Label a persistent task as interruptible to allow it to be restarted by `turbo watch`.
   * `turbo watch` watches for changes to your packages and automatically
   * restarts tasks that are affected. However, if a task is persistent, it will
   * not be restarted by default. To enable restarting persistent tasks, set
   * `interruptible` to true.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#interruptible
   *
   * @defaultValue `false`
   */
  interruptible?: boolean;

  /**
   * A list of tasks that will run alongside this task.
   *
   * Tasks in this list will not be run until completion before this task starts execution.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#with
   *
   * @defaultValue `[]`
   */
  with?: Array<string>;
}

"#
    .to_string()
}

/// Generate the BaseSchema interface
fn generate_base_schema_interface() -> String {
    r#"export interface BaseSchema {
  /** @defaultValue `https://turborepo.dev/schema.v2.json` */
  $schema?: string;
  /**
   * An object representing the task dependency graph of your project. turbo interprets
   * these conventions to schedule, execute, and cache the outputs of tasks in
   * your project.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#tasks
   *
   * @defaultValue `{}`
   */

  // eslint-disable-next-line @typescript-eslint/consistent-indexed-object-style -- it's more readable to specify a name for the key
  tasks: {
    /**
     * The name of a task that can be executed by turbo. If turbo finds a workspace
     * package with a package.json scripts object with a matching key, it will apply the
     * pipeline task configuration to that npm script during execution.
     */
    [script: string]: Pipeline;
  };
}

"#
    .to_string()
}

/// Generate the WorkspaceSchema interface
fn generate_workspace_schema_interface() -> String {
    r#"/** A `turbo.json` file in a package in the monorepo (not the root) */
export interface WorkspaceSchema extends BaseSchema {
  /**
   * This key is only available in Workspace Configs
   * and cannot be used in your root turbo.json.
   *
   * Tells turbo to extend your root `turbo.json`
   * and overrides with the keys provided
   * in your Workspace Configs.
   *
   * Currently, only the "//" value is allowed.
   *
   * @defaultValue `["//"]`
   */
  extends: Array<string>;
  /**
   * Used to tag a package for boundaries rules. Boundaries rules can restrict
   * which packages a tag group can import or be imported by.
   */
  tags?: Array<string>;
  /**
   * Configuration for `turbo boundaries` that is specific to this package
   */
  boundaries?: BoundariesConfig;
}

"#
    .to_string()
}

/// Generate the RootSchema interface
fn generate_root_schema_interface() -> String {
    r#"export interface RootSchema extends BaseSchema {
  /**
   * A list of globs to include in the set of implicit global hash dependencies.
   *
   * The contents of these files will be included in the global hashing
   * algorithm and affect the hashes of all tasks.
   *
   * This is useful for busting the cache based on:
   *
   * - .env files (not in Git)
   *
   * - any root level file that impacts package tasks
   * that are not represented in the traditional dependency graph
   * (e.g. a root tsconfig.json, jest.config.ts, .eslintrc, etc.)
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#globaldependencies
   *
   * @defaultValue `[]`
   */
  globalDependencies?: Array<string>;

  /**
   * A list of environment variables for implicit global hash dependencies.
   *
   * The variables included in this list will affect all task hashes.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#globalenv
   *
   * @defaultValue `[]`
   */
  globalEnv?: Array<EnvWildcard>;

  /**
   * An allowlist of environment variables that should be made to all tasks, but
   * should not contribute to the task's cache key, e.g. `AWS_SECRET_KEY`.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#globalpassthroughenv
   *
   * @defaultValue `null`
   */
  globalPassThroughEnv?: null | Array<EnvWildcard>;

  /**
   * Configuration options that control how turbo interfaces with the remote cache.
   *
   * Documentation: https://turborepo.dev/docs/core-concepts/remote-caching
   *
   * @defaultValue `{}`
   */
  remoteCache?: RemoteCache;

  /**
   * Enable use of the UI for `turbo`.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#ui
   *
   * @defaultValue `"stream"`
   */
  ui?: UI;

  /**
   * Set/limit the maximum concurrency for task execution. Must be an integer greater than or equal to `1` or a percentage value like `50%`.
   *
   *  - Use `1` to force serial execution (one task at a time).
   *  - Use `100%` to use all available logical processors.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#concurrency
   *
   * @defaultValue `"10"`
   */
  concurrency?: string;

  /**
   * Disable check for `packageManager` in root `package.json`
   *
   * This is highly discouraged as it leaves `turbo` dependent on system
   * configuration to infer the correct package manager.
   *
   * Some turbo features are disabled if this is set to true.
   *
   * @defaultValue `false`
   */
  dangerouslyDisablePackageManagerCheck?: boolean;

  /**
   * Specify the filesystem cache directory.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#cachedir
   *
   * @defaultValue `".turbo/cache"`
   */
  cacheDir?: RelativeUnixPath;

  /**
   * Turborepo runs a background process to pre-calculate some expensive operations. This standalone process (daemon) is a performance optimization, and not required for proper functioning of `turbo`.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#daemon
   *
   * @defaultValue `false`
   */
  daemon?: boolean;

  /**
   * Turborepo's Environment Modes allow you to control which environment variables are available to a task at runtime:
   *
   * - `"strict"`: Filter environment variables to only those that are specified in the `env` and `globalEnv` keys in `turbo.json`.
   * - `"loose"`: Allow all environment variables for the process to be available.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#envmode
   *
   * @defaultValue `"strict"`
   */
  envMode?: EnvMode;

  /**
   * Configuration for `turbo boundaries`. Allows users to restrict a package's dependencies and dependents
   */
  boundaries?: RootBoundariesConfig;

  /**
   * When set to `true`, disables the update notification that appears when a new version of `turbo` is available.
   *
   * Documentation: https://turborepo.dev/docs/reference/configuration#noupdatenotifier
   *
   * @defaultValue `false`
   */
  noUpdateNotifier?: boolean;

  /**
   * Opt into breaking changes prior to major releases, experimental features, and beta features.
   *
   * @defaultValue `{}`
   */
  futureFlags?: FutureFlags;
}

export interface FutureFlags {
  /**
   * When using `outputLogs: "errors-only"`, show task hashes when tasks
   * complete successfully. This provides visibility into which tasks are
   * running without showing full output logs.
   *
   * @defaultValue `false`
   */
  errorsOnlyShowHash?: boolean;
}

"#
    .to_string()
}

/// Add a type declaration with an optional doc comment
fn add_type_decl<T: TS>(output: &mut String, description: &str) {
    let decl = T::decl();
    if !decl.is_empty() {
        if !description.is_empty() {
            output.push_str(&format!("/** {} */\n", description));
        }
        output.push_str("export ");
        output.push_str(&decl);
        output.push('\n');
    }
}

/// Write output to file or stdout
fn write_output(content: &str, path: Option<&PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    match path {
        Some(path) => {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, content)?;
            eprintln!("Wrote output to {}", path.display());
        }
        None => {
            std::io::stdout().write_all(content.as_bytes())?;
        }
    }
    Ok(())
}

/// Verify that the existing schema matches what we would generate
fn verify_schema(path: &PathBuf) -> Result<bool, Box<dyn std::error::Error>> {
    let existing = fs::read_to_string(path)?;
    let generated = serde_json::to_string_pretty(&generate_schema())?;

    // Normalize both for comparison (parse and re-serialize)
    let existing_value: serde_json::Value = serde_json::from_str(&existing)?;
    let generated_value: serde_json::Value = serde_json::from_str(&generated)?;

    if existing_value == generated_value {
        eprintln!("Schema verification passed: {}", path.display());
        Ok(true)
    } else {
        eprintln!("Schema verification FAILED: {}", path.display());
        eprintln!("Generated schema differs from existing file.");
        eprintln!(
            "Run `turbo-schema-gen schema -o {}` to update.",
            path.display()
        );
        Ok(false)
    }
}

/// Verify that the existing TypeScript types match what we would generate
fn verify_typescript(path: &PathBuf) -> Result<bool, Box<dyn std::error::Error>> {
    let existing = fs::read_to_string(path)?;
    let generated = generate_typescript();

    // Simple string comparison (could be smarter about whitespace)
    if existing.trim() == generated.trim() {
        eprintln!("TypeScript verification passed: {}", path.display());
        Ok(true)
    } else {
        eprintln!("TypeScript verification FAILED: {}", path.display());
        eprintln!("Generated types differ from existing file.");
        eprintln!(
            "Run `turbo-schema-gen typescript -o {}` to update.",
            path.display()
        );
        Ok(false)
    }
}
