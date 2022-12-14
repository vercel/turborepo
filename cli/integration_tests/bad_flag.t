Setup
  $ . ${TESTDIR}/setup.sh

Bad flag should print misuse text
  $ ${TURBO} --bad-flag
  Repository inference failed: Unable to find `turbo.json` or `package.json` in current path
  Running command as global turbo
  error: Found argument '--bad-flag' which wasn't expected, or isn't valid in this context
  
    If you tried to supply '--bad-flag' as a value rather than a flag, use '-- --bad-flag'
  
  Usage: turbo [OPTIONS] [COMMAND]
  
  For more information try '--help'
  [1]

Bad flag with an implied run command should display run flags
  $ ${TURBO} build --bad-flag
  Repository inference failed: Unable to find `turbo.json` or `package.json` in current path
  Running command as global turbo
  error: Found argument '--bad-flag' which wasn't expected, or isn't valid in this context
  
    If you tried to supply '--bad-flag' as a value rather than a flag, use '-- --bad-flag'
  
  Usage: turbo <--cache-dir <CACHE_DIR>|--cache-workers <CACHE_WORKERS>|--concurrency <CONCURRENCY>|--continue|--dry-run [<DRY_RUN>]|--single-package|--filter <FILTER>|--force|--global-deps <GLOBAL_DEPS>|--graph [<GRAPH>]|--ignore <IGNORE>|--include-dependencies|--no-cache|--no-daemon|--no-deps|--output-logs <OUTPUT_LOGS>|--only|--parallel|--profile <PROFILE>|--remote-only|--scope <SCOPE>|--since <SINCE>|TASKS|PASS_THROUGH_ARGS>
  
  For more information try '--help'
  [1]
