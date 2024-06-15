Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh

  $ cp ${TESTDIR}/turbo.json $TARGET_DIR/turbo.json # overwrite
  $ git commit --quiet -am "Update turbo.json to include special inputs config"

Running build for my-app succeeds
  $ ${TURBO} run build --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing e228bd94fd46352c
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Update exluded file and try again
  $ echo "new excluded value" > apps/my-app/excluded.txt
  $ ${TURBO} run build --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs e228bd94fd46352c
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
