Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh inference/has_workspaces_dot_prefix

Test that workspaces with "./" prefix in package.json work correctly (GH-8599)
The fixture has: "workspaces": ["./apps/*", "./packages/*"]

Running from within a workspace directory should detect the monorepo (not single-package mode)
  $ cd $TARGET_DIR/apps/web && ${TURBO} run build
  \xe2\x80\xa2 Packages in scope: web (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  web:build: cache miss, executing [0-9a-f]+ (re)
  web:build: 
  web:build: > build
  web:build: > echo building web
  web:build: 
  web:build: building web
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[\.0-9]+m?s.* (re)
  

Filter by package name should work
  $ cd $TARGET_DIR && ${TURBO} run build --filter=ui
  \xe2\x80\xa2 Packages in scope: ui (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ui:build: cache miss, executing [0-9a-f]+ (re)
  ui:build: 
  ui:build: > build
  ui:build: > echo building ui
  ui:build: 
  ui:build: building ui
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[\.0-9]+m?s.* (re)
  

Filter with "./" prefix path should work
  $ cd $TARGET_DIR && ${TURBO} run build --filter=./packages/ui
  \xe2\x80\xa2 Packages in scope: ui (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ui:build: cache (miss, executing|hit, replaying logs) [0-9a-f]+ (re)
  ui:build: 
  ui:build: > build
  ui:build: > echo building ui
  ui:build: 
  ui:build: building ui
  
   Tasks:    1 successful, 1 total
  Cached:    [01] cached, 1 total (re)
    Time:\s+[\.0-9]+m?s.* (re)
  

Filter with "./" prefix path should work for another package
  $ cd $TARGET_DIR && ${TURBO} run build --filter=./apps/web
  \xe2\x80\xa2 Packages in scope: web (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  web:build: cache (miss, executing|hit, replaying logs) [0-9a-f]+ (re)
  web:build: 
  web:build: > build
  web:build: > echo building web
  web:build: 
  web:build: building web
  
   Tasks:    1 successful, 1 total
  Cached:    [01] cached, 1 total (re)
    Time:\s+[\.0-9]+m?s.* (re)
  
