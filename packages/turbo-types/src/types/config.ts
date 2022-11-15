/* This file generates the `schema.json` file. */
export interface Schema {
  /** @default https://turbo.build/schema.json */
  $schema?: string;

  /**
   * A list of globs for implicit global hash dependencies.
   *
   * The contents of these files will be included in the global hashing
   * algorithm and affect the hashes of all tasks.
   *
   * This is useful for busting the cache based on .env files (not in Git),
   * or any root level file that impacts package tasks (but are not represented
   * in the traditional dependency graph
   *
   * (e.g. a root tsconfig.json, jest.config.js, .eslintrc, etc.)).
   *
   * @default []
   */
  globalDependencies?: string[];

  /**
   * A list of environment variables, (e.g. GITHUB_TOKEN),
   * for implicit global hash dependencies.
   *
   * @default []
   */
  globalEnv?: string[];

  /**
   * An object representing the task dependency graph of your project. turbo interprets
   * these conventions to properly schedule, execute, and cache the outputs of tasks in
   * your project.
   *
   * @default {}
   */
  pipeline: {
    /**
     * The name of a task that can be executed by turbo run. If turbo finds a workspace
     * package with a package.json scripts object with a matching key, it will apply the
     * pipeline task configuration to that npm script during execution. This allows you to
     * use pipeline to set conventions across your entire Turborepo.
     */
    [script: string]: Pipeline;
  };
  /**
   * Configuration options that control how turbo interfaces with the remote Cache.
   * @default {}
   */
  remoteCache?: RemoteCache;
}

export interface Pipeline {
  /**
   * The list of tasks and environment variables that this task depends on.
   *
   * Prefixing an item in dependsOn with a ^ tells turbo that this pipeline task depends
   * on the package's topological dependencies completing the task with the ^ prefix first
   * (e.g. "a package's build tasks should only run once all of its dependencies and
   * devDependencies have completed their own build commands").
   *
   * Items in dependsOn without ^ prefix, express the relationships between tasks at the
   * package level (e.g. "a package's test and lint commands depend on build being
   * completed first").
   *
   * @default []
   */
  dependsOn?: string[];

  /**
   * A list of environment variables, **not** prefixed with $ (e.g. $GITHUB_TOKEN), that this task depends on.
   *
   * @default []
   */
  env?: string[];

  /**
   * The set of glob patterns indicating a task's cacheable filesystem outputs.
   *
   * Turborepo captures task logs for all tasks. This enables us to cache tasks whose runs
   * produce no artifacts other than logs (such as linters). Logs are always treated as a
   * cacheable artifact, and never need to be specified.
   *
   * @default []
   */
  outputs?: string[];

  /**
   * Whether or not to cache the task outputs. Setting cache to false is useful for daemon
   * or long-running "watch" or development mode tasks that you don't want to cache.
   *
   * @default true
   */
  cache?: boolean;

  /**
   * The set of glob patterns to consider as inputs to this task.
   *
   * Changes to files covered by these globs will cause a cache miss and force
   * the task to rerun. Changes to files in the package not covered by these globs
   * will not cause a cache miss.
   *
   * If omitted or empty, all files in the package are considered as inputs.
   * @default []
   */
  inputs?: string[];

  /**
   * The style of output for this task. Use "full" to display the entire output of
   * the task. Use "hash-only" to show only the computed task hashes. Use "new-only" to
   * show the full output of cache misses and the computed hashes for cache hits. Use
   * "none" to hide task output.
   *
   * @default full
   */
  outputMode?: string;
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
