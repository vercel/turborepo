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
      --concurrency <CONCURRENCY>
      --continue\[=<CONTINUE>\] (re)
      --single-package
      --framework-inference \[<BOOL>\] (re)
      --global-deps <GLOBAL_DEPS>
      --env-mode \[<ENV_MODE>\] (re)
      --filter <FILTER>
      --affected
      --output-logs <OUTPUT_LOGS>
      --log-order <LOG_ORDER>
      --only
      --pkg-inference-root <PKG_INFERENCE_ROOT>
      --log-prefix <LOG_PREFIX>
      TASKS
      PASS_THROUGH_ARGS
      --cache <CACHE>
      --force \[<FORCE>\] (re)
      --remote-only \[<REMOTE_ONLY>\] (re)
      --remote-cache-read-only \[<REMOTE_CACHE_READ_ONLY>\] (re)
      --no-cache <NO_CACHE>
      --cache-workers <CACHE_WORKERS>
      --dry-run \[<DRY_RUN>\] (re)
      --graph \[<GRAPH>\] (re)
      --daemon <DAEMON>
      --no-daemon <NO_DAEMON>
      --profile <PROFILE>
      --anon-profile <ANON_PROFILE>
      --summarize \[<SUMMARIZE>\] (re)
      --parallel <PARALLEL>
  
  For more information, try '--help'.
  
  [1]

