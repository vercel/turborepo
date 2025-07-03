Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh basic_monorepo
  $ mv turbo.json turborepo.json

Run without --root-turbo-json should fail
  $ ${TURBO} build
    x Could not find turbo.json or turbo.jsonc.
    | Follow directions at https://turborepo.com/docs to create one.
  
  [1]

Run with --root-turbo-json should use specified config
  $ ${TURBO} build --filter=my-app --root-turbo-json=turborepo.json
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing 0555ce94ca234049
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   WARNING  no output files found for task my-app#build. Please check your `outputs` key in `turbo.json`

Run with TURBO_ROOT_TURBO_JSON should use specified config
  $ TURBO_ROOT_TURBO_JSON=turborepo.json ${TURBO} build --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs 0555ce94ca234049
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  

Run with --continue
