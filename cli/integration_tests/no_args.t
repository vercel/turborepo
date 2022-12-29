Setup
  $ . ${TESTDIR}/setup.sh

Make sure exit code is 2 when no args are passed
  $ ${TURBO}
  Repository inference failed: Unable to find `turbo.json` or `package.json` in current path
  Running command as global turbo
  The build system that makes ship happen
  
  Usage: turbo [OPTIONS] [COMMAND]
  
  Commands:
    bin         Get the path to the Turbo binary
    completion  Generate the autocompletion script for the specified shell
    daemon      Runs the Turborepo background daemon
    link        Link your local directory to a Vercel organization and enable remote caching
    login       Login to your Vercel account
    logout      Logout to your Vercel account
    prune       Prepare a subset of your monorepo
    run         Run tasks across projects in your monorepo
    unlink      Unlink the current directory from your Vercel organization and disable Remote Caching
  
  Options:
        --version                   
        --skip-infer                Skip any attempts to infer which version of Turbo the project is configured to use
        --api <API>                 Override the endpoint for API calls
        --color                     Force color usage in the terminal
        --cpuprofile <CPU_PROFILE>  Specify a file to save a cpu profile
        --cwd <CWD>                 The directory in which to run turbo
        --heap <HEAP>               Specify a file to save a pprof heap profile
        --login <LOGIN>             Override the login endpoint
        --no-color                  Suppress color usage in the terminal
        --preflight                 When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --team <TEAM>               Set the team slug for API calls
        --token <TOKEN>             Set the auth token for API calls
        --trace <TRACE>             Specify a file to save a pprof trace
        --verbosity <COUNT>         Verbosity level
    -h, --help                      Print help information
  
  Run Arguments:
        --cache-dir <CACHE_DIR>          Override the filesystem cache directory
        --cache-workers <CACHE_WORKERS>  Set the number of concurrent cache operations (default 10) [default: 10]
        --concurrency <CONCURRENCY>      Limit the concurrency of task execution. Use 1 for serial (i.e. one-at-a-time) execution
        --continue                       Continue execution even if a task exits with an error or non-zero exit code. The default behavior is to bail
        --dry-run [<DRY_RUN>]            [possible values: text, json]
        --single-package                 Run turbo in single-package mode
        --filter <FILTER>                Use the given selector to specify package(s) to act as entry points. The syntax mirrors pnpm's syntax, and additional documentation and examples can be found in turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference#--filter
        --force                          Ignore the existing cache (to force execution)
        --global-deps <GLOBAL_DEPS>      Specify glob of global filesystem dependencies to be hashed. Useful for .env and files
        --graph [<GRAPH>]                Generate a graph of the task execution and output to a file when a filename is specified (.svg, .png, .jpg, .pdf, .json, .html). Outputs dot graph to stdout when if no filename is provided
        --ignore <IGNORE>                Files to ignore when calculating changed files (i.e. --since). Supports globs
        --include-dependencies           Include the dependencies of tasks in execution
        --no-cache                       Avoid saving task results to the cache. Useful for development/watch tasks
        --no-daemon                      Run without using turbo's daemon process
        --no-deps                        Exclude dependent task consumers from execution
        --output-logs <OUTPUT_LOGS>      Set type of process output logging. Use "full" to show all output. Use "hash-only" to show only turbo-computed task hashes. Use "new-only" to show only new output with only hashes for cached tasks. Use "none" to hide process output. (default full) [default: full] [possible values: full, none, hash-only, new-only, errors-only]
        --parallel                       Execute all tasks in parallel
        --profile <PROFILE>              File to write turbo's performance profile output into. You can load the file up in chrome://tracing to see which parts of your build were slow
        --remote-only                    Ignore the local filesystem cache for all tasks. Only allow reading and caching artifacts using the remote cache
        --scope <SCOPE>                  Specify package(s) to act as entry points for task execution. Supports globs
        --since <SINCE>                  Limit/Set scope to changed packages since a mergebase. This uses the git diff ${target_branch}... mechanism to identify which packages have changed
  [1]
