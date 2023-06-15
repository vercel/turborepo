Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) inputs

| Env var | Flag    | Result   |
| ------- | ------- | -------- |
| missing | missing | disabled |
| missing | true    | enabled  |
| missing | false   | disabled |
| true    | missing | enabled  |
| true    | true    | enabled  |
| true    | false   | disabled |
| false   | missing | disabled |
| false   | true    | enabled  |
| false   | false   | disabled |

Set team and token env vars, so that enabling disabling "Remote Only" can be tested.
Without this, remote caching cannot be enabled at all, so the config for Remote Only won't do anything..
  $ export TURBO_TOKEN="secrettoken"
  $ export TURBO_TEAM="myteam"

# force every build, we don't care about validating that caches are restored
  $ export TURBO_FORCE="true"

No env var or flag, means remote only is disabled
  $ ${TURBO} run build --output-logs=none | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: LOCAL, REMOTE (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
flag=true means remote only is enabled
  $ ${TURBO} run build --output-logs=none --remote-only=true | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: REMOTE,  (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
flag=false, means remote only is disabled
  $ ${TURBO} run build --output-logs=none --remote-only=false | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: LOCAL, REMOTE (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
envvar=true, means remote only is enabled
  $ TURBO_REMOTE_ONLY=true ${TURBO} run build --output-logs=none | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: REMOTE,  (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
envvar=true and flag=true, means remote only is enabled
  $ TURBO_REMOTE_ONLY=true ${TURBO} run build --output-logs=none --remote-only=true | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: REMOTE,  (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
envvar=true and flag=false, means remote only is disabled
  $ TURBO_REMOTE_ONLY=true ${TURBO} run build --output-logs=none --remote-only=false | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: LOCAL, REMOTE (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
envvar=false means remote only is disabled
  $ TURBO_REMOTE_ONLY=false ${TURBO} run build --output-logs=none | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: LOCAL, REMOTE (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
envvar=false and flag=true means remote only is enabled
  $ TURBO_REMOTE_ONLY=false ${TURBO} run build --output-logs=none --remote-only=true | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: REMOTE,  (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
envvar=false and flag=false means remote only is disabled
  $ TURBO_REMOTE_ONLY=false ${TURBO} run build --output-logs=none --remote-only=false | grep "Using caches:"
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Using caches: LOCAL, REMOTE (esc)
  \xe2\x80\xa2 Remote caching enabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  