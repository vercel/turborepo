Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

# Run all tests with --filter=util so we don't have any non-deterministic ordering

# run the first time to get basline hash
  $ ${TURBO} run build --filter=util --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing f99b445d4ff29bd0
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# run again and ensure there's a cache hit
  $ ${TURBO} run build --filter=util --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache hit, suppressing output f99b445d4ff29bd0
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# set global env var and ensure cache miss
  $ SOME_ENV_VAR=hi ${TURBO} run build --filter=util --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing 5b66e0d31ce27530
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# set env var with "THASH" and ensure cache miss
  $ SOMETHING_THASH_YES=hi ${TURBO} run build --filter=util --output-logs=hash-only
  [DEPRECATED] Using .*THASH.* to specify an environment variable for inclusion into the hash is deprecated. You specified: SOMETHING_THASH_YES.
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing e2dc82cc57d6c7f2
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# set vercel analytics env var and ensure cache miss
  $ VERCEL_ANALYTICS_ID=hi ${TURBO} run build --filter=util --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing a530620dd1982169
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# THASH deprecation doesn't break --dry=json
  $ SOMETHING_THASH_YES=hi ${TURBO} run build --filter=util --dry=json | jq -r '.tasks[0].environmentVariables.global[0]'
  SOMETHING_THASH_YES=8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4

# THASH deprecation doesn't break --graph
  $ SOMETHING_THASH_YES=hi ${TURBO} run build --filter=util --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
