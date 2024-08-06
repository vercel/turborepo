Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Make sure exit code is 2 when no args are passed
  $ ${TURBO}
  The build system that makes ship happen
  
  Usage: turbo(\.exe)? \[OPTIONS\] \[COMMAND\] (re)
  
  Commands:
    bin         Get the path to the Turbo binary
    completion  Generate the autocompletion script for the specified shell
    daemon      Runs the Turborepo background daemon
    generate    Generate a new app / package
    telemetry   Enable or disable anonymous telemetry
    scan        Turbo your monorepo by running a number of 'repo lints' to identify common issues, suggest fixes, and improve performance
    ls          EXPERIMENTAL: List packages in your monorepo
    link        Link your local directory to a Vercel organization and enable remote caching
    login       Login to your Vercel account
    logout      Logout to your Vercel account
    prune       Prepare a subset of your monorepo
    run         Run tasks across projects in your monorepo
    watch       Arguments used in run and watch
    unlink      Unlink the current directory from your Vercel organization and disable Remote Caching
  
  Options:
        --version
            
        --skip-infer
            Skip any attempts to infer which version of Turbo the project is configured to use
        --no-update-notifier
            Disable the turbo update notification
        --api <API>
            Override the endpoint for API calls
        --color
            Force color usage in the terminal
        --cwd <CWD>
            The directory in which to run turbo
        --heap <HEAP>
            Specify a file to save a pprof heap profile
        --ui <UI>
            Specify whether to use the streaming UI or TUI [possible values: tui, stream]
        --login <LOGIN>
            Override the login endpoint
        --no-color
            Suppress color usage in the terminal
        --preflight
            When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --remote-cache-timeout <TIMEOUT>
            Set a timeout for all HTTP requests
        --team <TEAM>
            Set the team slug for API calls
        --token <TOKEN>
            Set the auth token for API calls
        --trace <TRACE>
            Specify a file to save a pprof trace
        --verbosity <COUNT>
            Verbosity level
        --dangerously-disable-package-manager-check
            Allow for missing `packageManager` in `package.json`
    -h, --help
            Print help (see more with '--help')
  
  Run Arguments:
        --cache-workers <CACHE_WORKERS>
            Set the number of concurrent cache operations (default 10) [default: 10]
        --dry-run [<DRY_RUN>]
            [possible values: text, json]
        --graph [<GRAPH>]
            Generate a graph of the task execution and output to a file when a filename is specified (.svg, .png, .jpg, .pdf, .json, .html, .mermaid, .dot). Outputs dot graph to stdout when if no filename is provided
        --no-cache
            Avoid saving task results to the cache. Useful for development/watch tasks
        --[no-]daemon
            Force turbo to either use or not use the local daemon. If unset turbo will use the default detection logic
        --profile <PROFILE>
            File to write turbo's performance profile output into. You can load the file up in chrome://tracing to see which parts of your build were slow
        --anon-profile <ANON_PROFILE>
            File to write turbo's performance profile output into. All identifying data omitted from the profile
        --remote-cache-read-only [<BOOL>]
            Treat remote cache as read only [env: TURBO_REMOTE_CACHE_READ_ONLY=] [default: false] [possible values: true, false]
        --summarize [<SUMMARIZE>]
            Generate a summary of the turbo run [env: TURBO_RUN_SUMMARY=] [possible values: true, false]
        --parallel
            Execute all tasks in parallel
        --cache-dir <CACHE_DIR>
            Override the filesystem cache directory [env: TURBO_CACHE_DIR=]
        --concurrency <CONCURRENCY>
            Limit the concurrency of task execution. Use 1 for serial (i.e. one-at-a-time) execution
        --continue
            Continue execution even if a task exits with an error or non-zero exit code. The default behavior is to bail
        --single-package
            Run turbo in single-package mode
        --force [<FORCE>]
            Ignore the existing cache (to force execution) [env: TURBO_FORCE=] [possible values: true, false]
        --framework-inference [<BOOL>]
            Specify whether or not to do framework inference for tasks [default: true] [possible values: true, false]
        --global-deps <GLOBAL_DEPS>
            Specify glob of global filesystem dependencies to be hashed. Useful for .env and files
        --env-mode [<ENV_MODE>]
            Environment variable mode. Use "loose" to pass the entire existing environment. Use "strict" to use an allowlist specified in turbo.json [possible values: loose, strict]
    -F, --filter <FILTER>
            Use the given selector to specify package(s) to act as entry points. The syntax mirrors pnpm's syntax, and additional documentation and examples can be found in turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference/run#--filter
        --affected
            Run only tasks that are affected by changes between the current branch and `main`
        --output-logs <OUTPUT_LOGS>
            Set type of process output logging. Use "full" to show all output. Use "hash-only" to show only turbo-computed task hashes. Use "new-only" to show only new output with only hashes for cached tasks. Use "none" to hide process output. (default full) [possible values: full, none, hash-only, new-only, errors-only]
        --log-order <LOG_ORDER>
            Set type of task output order. Use "stream" to show output as soon as it is available. Use "grouped" to show output when a command has finished execution. Use "auto" to let turbo decide based on its own heuristics. (default auto) [env: TURBO_LOG_ORDER=] [default: auto] [possible values: auto, stream, grouped]
        --only
            Only executes the tasks specified, does not execute parent tasks
        --remote-only [<BOOL>]
            Ignore the local filesystem cache for all tasks. Only allow reading and caching artifacts using the remote cache [env: TURBO_REMOTE_ONLY=] [default: false] [possible values: true, false]
        --log-prefix <LOG_PREFIX>
            Use "none" to remove prefixes from task logs. Use "task" to get task id prefixing. Use "auto" to let turbo decide how to prefix the logs based on the execution environment. In most cases this will be the same as "task". Note that tasks running in parallel interleave their logs, so removing prefixes can make it difficult to associate logs with tasks. Use --log-order=grouped to prevent interleaving. (default auto) [default: auto] [possible values: auto, none, task]
  [1]

Run without any tasks, get a list of potential tasks to run
  $ ${TURBO} run
  No tasks provided, here are some potential ones to run
  
    build
      my-app, util
    maybefails
      my-app, util
  [1]

Run again with a filter and get only the packages that match
  $ ${TURBO} run --filter my-app
  No tasks provided, here are some potential ones to run
  
    build
      my-app
    maybefails
      my-app
  [1]


Run again with an environment variable that corresponds to a run argument and assert that
we get the full help output.
  $ TURBO_LOG_ORDER=stream ${TURBO} 2>&1 > out.txt
  [1]
  $ cat out.txt | head -n1
  The build system that makes ship happen

Initialize a new monorepo
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh composable_config > /dev/null 2>&1

  $ ${TURBO} run
  No tasks provided, here are some potential ones to run
  
    build
      invalid-config, my-app, util
    maybefails
      my-app, util
    add-keys-task
      add-keys
    add-keys-underlying-task
      add-keys
    added-task
      add-tasks
    cached-task-1
      cached
    cached-task-2
      cached
    cached-task-3
      cached
    cached-task-4
      missing-workspace-config
    config-change-task
      config-change
    cross-workspace-task
      cross-workspace
    cross-workspace-underlying-task
      blank-pkg
    missing-workspace-config-task
      missing-workspace-config
    missing-workspace-config-task-with-deps
      missing-workspace-config
    missing-workspace-config-underlying-task
      missing-workspace-config
    missing-workspace-config-underlying-topo-task
      blank-pkg
    omit-keys-task
      omit-keys
    omit-keys-task-with-deps
      omit-keys
    omit-keys-underlying-task
      omit-keys
    omit-keys-underlying-topo-task
      blank-pkg
    override-values-task
      override-values
    override-values-task-with-deps
      override-values
    override-values-task-with-deps-2
      override-values
    override-values-underlying-task
      override-values
    override-values-underlying-topo-task
      blank-pkg
    persistent-task-1
      persistent
    persistent-task-1-parent
      persistent
    persistent-task-2
      persistent
    persistent-task-2-parent
      persistent
    persistent-task-3
      persistent
    persistent-task-3-parent
      persistent
    persistent-task-4
      persistent
    persistent-task-4-parent
      persistent
    trailing-comma
      bad-json
  [1]
