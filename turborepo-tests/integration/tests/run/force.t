Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

# Tests
| env var | flag    | bypass? |
| ------- | ------- | ------- |
| true    | missing | yes     |
| true    | true    | yes     |
| true    | false   | no      |
| true    | novalue | yes     |

| false   | missing | no      |
| false   | true    | yes     |
| false   | false   | no      |
| false   | novalue | yes     |

| missing | missing | no      |
| missing | true    | yes     |
| missing | false   | no      |
| missing | novalue | yes     |

baseline to generate cache
  $ ${TURBO} run build --output-logs=hash-only --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

# env var=true, missing flag: cache bypass
  $ TURBO_FORCE=true ${TURBO} run build --output-logs=hash-only --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# env var=true, --flag=true: cache bypass
  $ TURBO_FORCE=true ${TURBO} run build --output-logs=hash-only --filter=my-app --force=true
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# env var=true, --flag=false: cache hit
  $ TURBO_FORCE=true ${TURBO} run build --output-logs=hash-only --filter=my-app --force=false
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, suppressing output 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# env var=true, --flag (no value): cache bypass
  $ TURBO_FORCE=true ${TURBO} run build --output-logs=hash-only --filter=my-app --force
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

# env var=false, missing flag, cache hit
  $ TURBO_FORCE=false ${TURBO} run build --output-logs=hash-only --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, suppressing output 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# env var=false, --flag=true: cache bypass
  $ TURBO_FORCE=false ${TURBO} run build --output-logs=hash-only --filter=my-app --force=true
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# env var=false, --flag=false: cache hit
  $ TURBO_FORCE=false ${TURBO} run build --output-logs=hash-only --filter=my-app --force=false
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, suppressing output 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# env var=false, --flag (no value): cache bypass
  $ TURBO_FORCE=false ${TURBO} run build --output-logs=hash-only --filter=my-app --force
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

# missing env var, missing flag: cache hit
  $ ${TURBO} run build --output-logs=hash-only --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, suppressing output 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# missing env var, --flag=true: cache bypass
  $ ${TURBO} run build --output-logs=hash-only --filter=my-app --force=true
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# missing env var, --flag=false: cache hit
  $ ${TURBO} run build --output-logs=hash-only --filter=my-app --force=false
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, suppressing output 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# missing env var, --flag (no value): cache bypass
  $ ${TURBO} run build --output-logs=hash-only --filter=my-app --force
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 2f192ed93e20f940
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
