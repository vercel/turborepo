Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

Test help flag
  $ ${TURBO} -h
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
        --skip-infer                      Skip any attempts to infer which version of Turbo the project is configured to use
        --no-update-notifier              Disable the turbo update notification
        --api <API>                       Override the endpoint for API calls
        --color                           Force color usage in the terminal
        --cpuprofile <CPU_PROFILE>        Specify a file to save a cpu profile
        --cwd <CWD>                       The directory in which to run turbo
        --heap <HEAP>                     Specify a file to save a pprof heap profile
        --login <LOGIN>                   Override the login endpoint
        --no-color                        Suppress color usage in the terminal
        --preflight                       When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --remote-cache-timeout <TIMEOUT>  Set a timeout for all HTTP requests
        --team <TEAM>                     Set the team slug for API calls
        --token <TOKEN>                   Set the auth token for API calls
        --trace <TRACE>                   Specify a file to save a pprof trace
        --verbosity <COUNT>               Verbosity level
    -h, --help                            Print help
  
  Run Arguments:
        --cache-dir <CACHE_DIR>          Override the filesystem cache directory
        --cache-workers <CACHE_WORKERS>  Set the number of concurrent cache operations (default 10) [default: 10]
        --concurrency <CONCURRENCY>      Limit the concurrency of task execution. Use 1 for serial (i.e. one-at-a-time) execution
        --continue                       Continue execution even if a task exits with an error or non-zero exit code. The default behavior is to bail
        --dry-run [<DRY_RUN>]            [possible values: text, json]
        --single-package                 Run turbo in single-package mode
    -F, --filter <FILTER>                Use the given selector to specify package(s) to act as entry points. The syntax mirrors pnpm's syntax, and additional documentation and examples can be found in turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference#--filter
        --force [<FORCE>]                Ignore the existing cache (to force execution) [env: TURBO_FORCE=] [possible values: true, false]
        --global-deps <GLOBAL_DEPS>      Specify glob of global filesystem dependencies to be hashed. Useful for .env and files
        --graph [<GRAPH>]                Generate a graph of the task execution and output to a file when a filename is specified (.svg, .png, .jpg, .pdf, .json, .html). Outputs dot graph to stdout when if no filename is provided
        --ignore <IGNORE>                Files to ignore when calculating changed files (i.e. --since). Supports globs
        --include-dependencies           Include the dependencies of tasks in execution
        --no-cache                       Avoid saving task results to the cache. Useful for development/watch tasks
        --no-daemon                      Run without using turbo's daemon process
        --no-deps                        Exclude dependent task consumers from execution
        --output-logs <OUTPUT_LOGS>      Set type of process output logging. Use "full" to show all output. Use "hash-only" to show only turbo-computed task hashes. Use "new-only" to show only new output with only hashes for cached tasks. Use "none" to hide process output. (default full) [possible values: full, none, hash-only, new-only, errors-only]
        --parallel                       Execute all tasks in parallel
        --profile <PROFILE>              File to write turbo's performance profile output into. You can load the file up in chrome://tracing to see which parts of your build were slow
        --remote-only                    Ignore the local filesystem cache for all tasks. Only allow reading and caching artifacts using the remote cache
        --scope <SCOPE>                  Specify package(s) to act as entry points for task execution. Supports globs
        --since <SINCE>                  Limit/Set scope to changed packages since a mergebase. This uses the git diff ${target_branch}... mechanism to identify which packages have changed
        --summarize [<SUMMARIZE>]        Generate a summary of the turbo run [env: TURBO_RUN_SUMMARY=] [possible values: true, false]
        --log-prefix <LOG_PREFIX>        Use "none" to remove prefixes from task logs. Note that tasks running in parallel interleave their logs and prefix is the only way to identify which task produced a log [possible values: none]






  $ ${TURBO} --help
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
        --skip-infer                      Skip any attempts to infer which version of Turbo the project is configured to use
        --no-update-notifier              Disable the turbo update notification
        --api <API>                       Override the endpoint for API calls
        --color                           Force color usage in the terminal
        --cpuprofile <CPU_PROFILE>        Specify a file to save a cpu profile
        --cwd <CWD>                       The directory in which to run turbo
        --heap <HEAP>                     Specify a file to save a pprof heap profile
        --login <LOGIN>                   Override the login endpoint
        --no-color                        Suppress color usage in the terminal
        --preflight                       When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --remote-cache-timeout <TIMEOUT>  Set a timeout for all HTTP requests
        --team <TEAM>                     Set the team slug for API calls
        --token <TOKEN>                   Set the auth token for API calls
        --trace <TRACE>                   Specify a file to save a pprof trace
        --verbosity <COUNT>               Verbosity level
    -h, --help                            Print help
  
  Run Arguments:
        --cache-dir <CACHE_DIR>          Override the filesystem cache directory
        --cache-workers <CACHE_WORKERS>  Set the number of concurrent cache operations (default 10) [default: 10]
        --concurrency <CONCURRENCY>      Limit the concurrency of task execution. Use 1 for serial (i.e. one-at-a-time) execution
        --continue                       Continue execution even if a task exits with an error or non-zero exit code. The default behavior is to bail
        --dry-run [<DRY_RUN>]            [possible values: text, json]
        --single-package                 Run turbo in single-package mode
    -F, --filter <FILTER>                Use the given selector to specify package(s) to act as entry points. The syntax mirrors pnpm's syntax, and additional documentation and examples can be found in turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference#--filter
        --force [<FORCE>]                Ignore the existing cache (to force execution) [env: TURBO_FORCE=] [possible values: true, false]
        --global-deps <GLOBAL_DEPS>      Specify glob of global filesystem dependencies to be hashed. Useful for .env and files
        --graph [<GRAPH>]                Generate a graph of the task execution and output to a file when a filename is specified (.svg, .png, .jpg, .pdf, .json, .html). Outputs dot graph to stdout when if no filename is provided
        --ignore <IGNORE>                Files to ignore when calculating changed files (i.e. --since). Supports globs
        --include-dependencies           Include the dependencies of tasks in execution
        --no-cache                       Avoid saving task results to the cache. Useful for development/watch tasks
        --no-daemon                      Run without using turbo's daemon process
        --no-deps                        Exclude dependent task consumers from execution
        --output-logs <OUTPUT_LOGS>      Set type of process output logging. Use "full" to show all output. Use "hash-only" to show only turbo-computed task hashes. Use "new-only" to show only new output with only hashes for cached tasks. Use "none" to hide process output. (default full) [possible values: full, none, hash-only, new-only, errors-only]
        --parallel                       Execute all tasks in parallel
        --profile <PROFILE>              File to write turbo's performance profile output into. You can load the file up in chrome://tracing to see which parts of your build were slow
        --remote-only                    Ignore the local filesystem cache for all tasks. Only allow reading and caching artifacts using the remote cache
        --scope <SCOPE>                  Specify package(s) to act as entry points for task execution. Supports globs
        --since <SINCE>                  Limit/Set scope to changed packages since a mergebase. This uses the git diff ${target_branch}... mechanism to identify which packages have changed
        --summarize [<SUMMARIZE>]        Generate a summary of the turbo run [env: TURBO_RUN_SUMMARY=] [possible values: true, false]
        --log-prefix <LOG_PREFIX>        Use "none" to remove prefixes from task logs. Note that tasks running in parallel interleave their logs and prefix is the only way to identify which task produced a log [possible values: none]

Test help flag for link command
  $ ${TURBO} link -h
  Link your local directory to a Vercel organization and enable remote caching
  
  Usage: turbo link [OPTIONS]
  
  Options:
        --no-gitignore                    Do not create or modify .gitignore (default false)
        --version                         
        --skip-infer                      Skip any attempts to infer which version of Turbo the project is configured to use
        --target <TARGET>                 Specify what should be linked (default "remote cache") [default: remote-cache] [possible values: remote-cache, spaces]
        --no-update-notifier              Disable the turbo update notification
        --api <API>                       Override the endpoint for API calls
        --color                           Force color usage in the terminal
        --cpuprofile <CPU_PROFILE>        Specify a file to save a cpu profile
        --cwd <CWD>                       The directory in which to run turbo
        --heap <HEAP>                     Specify a file to save a pprof heap profile
        --login <LOGIN>                   Override the login endpoint
        --no-color                        Suppress color usage in the terminal
        --preflight                       When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --remote-cache-timeout <TIMEOUT>  Set a timeout for all HTTP requests
        --team <TEAM>                     Set the team slug for API calls
        --token <TOKEN>                   Set the auth token for API calls
        --trace <TRACE>                   Specify a file to save a pprof trace
        --verbosity <COUNT>               Verbosity level
    -h, --help                            Print help
  
  Run Arguments:
        --single-package  Run turbo in single-package mode

Test help flag for unlink command
  $ ${TURBO} unlink -h
  Unlink the current directory from your Vercel organization and disable Remote Caching
  
  Usage: turbo unlink [OPTIONS]
  
  Options:
        --target <TARGET>                 Specify what should be unlinked (default "remote cache") [default: remote-cache] [possible values: remote-cache, spaces]
        --version                         
        --skip-infer                      Skip any attempts to infer which version of Turbo the project is configured to use
        --no-update-notifier              Disable the turbo update notification
        --api <API>                       Override the endpoint for API calls
        --color                           Force color usage in the terminal
        --cpuprofile <CPU_PROFILE>        Specify a file to save a cpu profile
        --cwd <CWD>                       The directory in which to run turbo
        --heap <HEAP>                     Specify a file to save a pprof heap profile
        --login <LOGIN>                   Override the login endpoint
        --no-color                        Suppress color usage in the terminal
        --preflight                       When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --remote-cache-timeout <TIMEOUT>  Set a timeout for all HTTP requests
        --team <TEAM>                     Set the team slug for API calls
        --token <TOKEN>                   Set the auth token for API calls
        --trace <TRACE>                   Specify a file to save a pprof trace
        --verbosity <COUNT>               Verbosity level
    -h, --help                            Print help
  
  Run Arguments:
        --single-package  Run turbo in single-package mode

Test help flag for login command
  $ ${TURBO} login -h
  Login to your Vercel account
  
  Usage: turbo login [OPTIONS]
  
  Options:
        --sso-team <SSO_TEAM>             
        --version                         
        --skip-infer                      Skip any attempts to infer which version of Turbo the project is configured to use
        --no-update-notifier              Disable the turbo update notification
        --api <API>                       Override the endpoint for API calls
        --color                           Force color usage in the terminal
        --cpuprofile <CPU_PROFILE>        Specify a file to save a cpu profile
        --cwd <CWD>                       The directory in which to run turbo
        --heap <HEAP>                     Specify a file to save a pprof heap profile
        --login <LOGIN>                   Override the login endpoint
        --no-color                        Suppress color usage in the terminal
        --preflight                       When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --remote-cache-timeout <TIMEOUT>  Set a timeout for all HTTP requests
        --team <TEAM>                     Set the team slug for API calls
        --token <TOKEN>                   Set the auth token for API calls
        --trace <TRACE>                   Specify a file to save a pprof trace
        --verbosity <COUNT>               Verbosity level
    -h, --help                            Print help
  
  Run Arguments:
        --single-package  Run turbo in single-package mode

Test help flag for logout command
  $ ${TURBO} logout -h
  Logout to your Vercel account
  
  Usage: turbo logout [OPTIONS]
  
  Options:
        --version                         
        --skip-infer                      Skip any attempts to infer which version of Turbo the project is configured to use
        --no-update-notifier              Disable the turbo update notification
        --api <API>                       Override the endpoint for API calls
        --color                           Force color usage in the terminal
        --cpuprofile <CPU_PROFILE>        Specify a file to save a cpu profile
        --cwd <CWD>                       The directory in which to run turbo
        --heap <HEAP>                     Specify a file to save a pprof heap profile
        --login <LOGIN>                   Override the login endpoint
        --no-color                        Suppress color usage in the terminal
        --preflight                       When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --remote-cache-timeout <TIMEOUT>  Set a timeout for all HTTP requests
        --team <TEAM>                     Set the team slug for API calls
        --token <TOKEN>                   Set the auth token for API calls
        --trace <TRACE>                   Specify a file to save a pprof trace
        --verbosity <COUNT>               Verbosity level
    -h, --help                            Print help
  
  Run Arguments:
        --single-package  Run turbo in single-package mode
