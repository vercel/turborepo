export type OutputLogs =
  | "full"
  | "hash-only"
  | "new-only"
  | "errors-only"
  | "none";
export type EnvMode = "strict" | "loose";
export type UI = "tui" | "stream";

/**
 * This is a relative Unix-style path (e.g. `./src/index.ts` or `src/index.ts`).  Absolute paths (e.g. `/tmp/foo`) are not valid.
 */
export type RelativeUnixPath = string;
export type EnvWildcard = string;

export interface BaseSchema {
  /** @defaultValue `https://turbo.build/schema.v2.json` */
  $schema?: string;
  /**
   * An object representing the task dependency graph of your project. turbo interprets
   * these conventions to schedule, execute, and cache the outputs of tasks in
   * your project.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#tasks
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

/** A `turbo.json` file in a package in the monorepo (not the root) */
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
}

export interface RootSchema extends BaseSchema {
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
   * (e.g. a root tsconfig.json, jest.config.js, .eslintrc, etc.)
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#globaldependencies
   *
   * @defaultValue `[]`
   */
  globalDependencies?: Array<string>;

  /**
   * A list of environment variables for implicit global hash dependencies.
   *
   * The variables included in this list will affect all task hashes.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#globalenv
   *
   * @defaultValue `[]`
   */
  globalEnv?: Array<EnvWildcard>;

  /**
   * An allowlist of environment variables that should be made to all tasks, but
   * should not contribute to the task's cache key, e.g. `AWS_SECRET_KEY`.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#globalpassthroughenv
   *
   * @defaultValue `null`
   */
  globalPassThroughEnv?: null | Array<EnvWildcard>;

  /**
   * Configuration options that control how turbo interfaces with the remote cache.
   *
   * Documentation: https://turbo.build/repo/docs/core-concepts/remote-caching
   *
   * @defaultValue `{}`
   */
  remoteCache?: RemoteCache;

  /**
   * Enable use of the UI for `turbo`.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#ui
   *
   * @defaultValue `"stream"`
   */
  ui?: UI;

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
   * Documentation: https://turbo.build/repo/docs/reference/configuration#cachedir
   *
   * @defaultValue `".turbo/cache"`
   */
  cacheDir?: RelativeUnixPath;

  /**
   * Turborepo runs a background process to pre-calculate some expensive operations. This standalone process (daemon) is a performance optimization, and not required for proper functioning of `turbo`.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#daemon
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
   * Documentation: https://turbo.build/repo/docs/reference/configuration#envmode
   *
   * @defaultValue `"strict"`
   */
  envMode?: EnvMode;
}

export interface Pipeline {
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
   * Documentation: https://turbo.build/repo/docs/reference/configuration#dependson
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
   * Documentation: https://turbo.build/repo/docs/reference/configuration#env
   *
   * @defaultValue `[]`
   */
  env?: Array<EnvWildcard>;

  /**
   * An allowlist of environment variables that should be made available in this
   * task's environment, but should not contribute to the task's cache key,
   * e.g. `AWS_SECRET_KEY`.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#passthroughenv
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
   * Documentation: https://turbo.build/repo/docs/reference/configuration#outputs
   *
   * @defaultValue `[]`
   */
  outputs?: Array<string>;

  /**
   * Whether or not to cache the outputs of the task.
   *
   * Setting cache to false is useful for long-running "watch" or development mode tasks.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#cache
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
   * Documentation: https://turbo.build/repo/docs/reference/configuration#inputs
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
   * Documentation: https://turbo.build/repo/docs/reference/run#--output-logs-option
   *
   * @defaultValue `"full"`
   */
  outputLogs?: OutputLogs;

  /**
   * Indicates whether the task exits or not. Setting `persistent` to `true` tells
   * turbo that this is a long-running task and will ensure that other tasks
   * cannot depend on it.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#persistent
   *
   * @defaultValue `false`
   */
  persistent?: boolean;

  /**
   * Mark a task as interactive allowing it to receive input from stdin.
   * Interactive tasks must be marked with "cache": false as the input
   * they receive from stdin can change the outcome of the task.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#interactive
   *
   * @defaultValue `false`
   */
  interactive?: boolean;
}

export interface RemoteCache {
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
   * Documentation: https://turbo.build/repo/docs/core-concepts/remote-caching
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
   * Documentation: https://turbo.build/repo/docs/core-concepts/remote-caching#self-hosting
   *
   * @defaultValue `"https://vercel.com/api"`
   */
  apiUrl?: string;
  /**
   * Set endpoint for requesting tokens during `turbo login`.
   * Documentation: https://turbo.build/repo/docs/core-concepts/remote-caching#self-hosting
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
}

export const isRootSchemaV2 = (schema: Schema): schema is RootSchema =>
  !("extends" in schema);

export const isWorkspaceSchemaV2 = (
  schema: Schema
): schema is WorkspaceSchema => !isRootSchemaV2(schema);

export type Schema = RootSchema | WorkspaceSchema;
