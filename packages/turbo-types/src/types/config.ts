/* This file generates the `schema.json` file. */
export type Schema = RootSchema | WorkspaceSchema;

export interface BaseSchema {
  /** @default https://turbo.build/schema.json */
  $schema?: string;
  /**
   * An object representing the task dependency graph of your project. turbo interprets
   * these conventions to schedule, execute, and cache the outputs of tasks in
   * your project.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#pipeline
   *
   * @default {}
   */
  pipeline: {
    /**
     * The name of a task that can be executed by turbo. If turbo finds a workspace
     * package with a package.json scripts object with a matching key, it will apply the
     * pipeline task configuration to that npm script during execution.
     */
    [script: string]: Pipeline;
  };
}

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
   * @default ["//"]
   */
  extends: string[];
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
   * @default []
   */
  globalDependencies?: string[];

  /**
   * A list of environment variables for implicit global hash dependencies.
   *
   * The variables included in this list will affect all task hashes.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#globalenv
   *
   * @default []
   */
  globalEnv?: string[];

  /**
   * An allowlist of environment variables that should be made to all tasks, but
   * should not contribute to the task's cache key, e.g. `AWS_SECRET_KEY`.
   *
   * Only applies in `--env=strict` mode.
   *
   * Documentation: https://turbo.build/repo/docs/refernce/configuration#experimentalGlobalPassThroughEnv
   *
   * @default []
   */
  experimentalGlobalPassThroughEnv?: string[];

  /**
   * Configuration options that control how turbo interfaces with the remote cache.
   *
   * Documentation: https://turbo.build/repo/docs/core-concepts/remote-caching
   *
   * @default {}
   */
  remoteCache?: RemoteCache;
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
   * @default []
   */
  dependsOn?: string[];

  /**
   * A list of environment variables that this task depends on.
   *
   * Note: If you are migrating from a turbo version 1.5 or below,
   * you may be used to prefixing your variables with a $.
   * You no longer need to use the $ prefix.
   * (e.g. $GITHUB_TOKEN -> GITHUB_TOKEN)
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#env
   *
   * @default []
   */
  env?: string[];

  /**
   * An allowlist of environment variables that should be made available in this
   * task's environment, but should not contribute to the task's cache key,
   * e.g. `AWS_SECRET_KEY`.
   *
   * Only applies in `--env=strict` mode.
   *
   * Documentation: https://turbo.build/repo/docs/refernce/configuration#experimentalPassThroughEnv
   *
   * @default []
   */
  experimentalPassThroughEnv?: string[];

  /**
   * The set of glob patterns indicating a task's cacheable filesystem outputs.
   *
   * Turborepo captures task logs for all tasks. This enables us to cache tasks whose runs
   * produce no artifacts other than logs (such as linters). Logs are always treated as a
   * cacheable artifact and never need to be specified.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#outputs
   *
   * @default []
   */
  outputs?: string[];

  /**
   * Whether or not to cache the outputs of the task.
   *
   * Setting cache to false is useful for long-running "watch" or development mode tasks.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#cache
   *
   * @default true
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
   * @default []
   */
  inputs?: string[];

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
   * Documentation: https://turbo.build/repo/docs/reference/command-line-reference#--output-logs
   *
   * @default full
   */
  outputMode?: OutputMode;

  /**
   * Indicates whether the task exits or not. Setting `persistent` to `true` tells
   * turbo that this is a long-running task and will ensure that other tasks
   * cannot depend on it.
   *
   * Documentation: https://turbo.build/repo/docs/reference/configuration#persistent
   *
   * @default false
   */
  persistent?: boolean;
}

export interface RemoteCache {
  /**
   * Indicates if signature verification is enabled for requests to the remote cache. When
   * `true`, Turborepo will sign every uploaded artifact using the value of the environment
   * variable `TURBO_REMOTE_CACHE_SIGNATURE_KEY`. Turborepo will reject any downloaded artifacts
   * that have an invalid signature or are missing a signature.
   *
   * @default false
   */
  signature?: boolean;
}

export type OutputMode =
  | "full"
  | "hash-only"
  | "new-only"
  | "errors-only"
  | "none";
