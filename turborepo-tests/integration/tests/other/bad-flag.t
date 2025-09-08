Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh

Bad flag should print misuse text
  $ ${TURBO} --bad-flag
   ERROR  unexpected argument '--bad-flag' found
  
    tip: to pass '--bad-flag' as a value, use '-- --bad-flag'
  
  Usage: turbo(\.exe)? \[OPTIONS\] \[COMMAND\] (re)
  
  For more information, try '--help'.
  
  [1]

Bad flag with an implied run command should display run flags
  $ ${TURBO} build --bad-flag
   ERROR  unexpected argument '--bad-flag' found
  
    tip: to pass '--bad-flag' as a value, use '-- --bad-flag'
  
  Usage: turbo(\.exe)? \[OPTIONS\] \[TASKS\]... \[-- <PASS_THROUGH_ARGS>...\] (re)
  
  Options:
      --cache-dir <CACHE_DIR>
      --cache-workers <CACHE_WORKERS>
      --concurrency <CONCURRENCY>
      --continue
      --dry-run \[<DRY_RUN>\] (re)
      --single-package
      --filter <FILTER>
      --force \[<FORCE>\] (re)
      --framework-inference \[<BOOL>\] (re)
      --global-deps <GLOBAL_DEPS>
      --graph \[<GRAPH>\] (re)
      --env-mode \[<ENV_MODE>\] (re)
      --ignore <IGNORE>
      --no-cache
      --no-daemon
      --output-logs <OUTPUT_LOGS>
      --log-order <LOG_ORDER>
      --only
      --parallel
      --pkg-inference-root <PKG_INFERENCE_ROOT>
      --profile <PROFILE>
      --remote-only \[<BOOL>\] (re)
      --summarize \[<SUMMARIZE>\] (re)
      --log-prefix <LOG_PREFIX>
      TASKS
      PASS_THROUGH_ARGS
  
  For more information, try '--help'.
  
  [1]

