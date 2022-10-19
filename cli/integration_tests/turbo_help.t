Setup
  $ . ${TESTDIR}/setup.sh

Test help flag
  $ ${TURBO} -h
  The task runner that makes ship happen
  
  Usage:
    turbo [command]
  
  Available Commands:
    bin         Get the path to the Turbo binary
    completion  Generate the autocompletion script for the specified shell
    daemon      Runs the Turborepo background daemon
    help        Help about any command
    link        Link your local directory to a Vercel organization and enable remote caching.
    login       Login to your Vercel account
    logout      Logout of your Vercel account
    prune       Prepare a subset of your monorepo.
    run         Run tasks across projects in your monorepo
    unlink      Unlink the current directory from your Vercel organization and disable Remote Caching
  
  Flags:
        --api string          Override the endpoint for API calls
        --color               Force color usage in the terminal
        --cpuprofile string   Specify a file to save a cpu profile
        --cwd string          The directory in which to run turbo
        --heap string         Specify a file to save a pprof heap profile
    -h, --help                help for turbo
        --login string        Override the login endpoint
        --no-color            Suppress color usage in the terminal
        --preflight           When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --team string         Set the team slug for API calls
        --token string        Set the auth token for API calls
        --trace string        Specify a file to save a pprof trace
    -v, --verbosity count     verbosity
        --version             version for turbo
  
  Use "turbo [command] --help" for more information about a command.

  $ ${TURBO} --help
  The task runner that makes ship happen
  
  Usage:
    turbo [command]
  
  Available Commands:
    bin         Get the path to the Turbo binary
    completion  Generate the autocompletion script for the specified shell
    daemon      Runs the Turborepo background daemon
    help        Help about any command
    link        Link your local directory to a Vercel organization and enable remote caching.
    login       Login to your Vercel account
    logout      Logout of your Vercel account
    prune       Prepare a subset of your monorepo.
    run         Run tasks across projects in your monorepo
    unlink      Unlink the current directory from your Vercel organization and disable Remote Caching
  
  Flags:
        --api string          Override the endpoint for API calls
        --color               Force color usage in the terminal
        --cpuprofile string   Specify a file to save a cpu profile
        --cwd string          The directory in which to run turbo
        --heap string         Specify a file to save a pprof heap profile
    -h, --help                help for turbo
        --login string        Override the login endpoint
        --no-color            Suppress color usage in the terminal
        --preflight           When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
        --team string         Set the team slug for API calls
        --token string        Set the auth token for API calls
        --trace string        Specify a file to save a pprof trace
    -v, --verbosity count     verbosity
        --version             version for turbo
  
  Use "turbo [command] --help" for more information about a command.
