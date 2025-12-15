Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

Test that task-level extends: false excludes a task from inheritance.
The workspace turbo.json has "lint": { "extends": false }, so lint should not run.

Running build should work (inherited normally)
  $ ${TURBO} run build --filter=task-extends-exclude
  \xe2\x80\xa2 Packages in scope: task-extends-exclude (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  task-extends-exclude:build: cache miss, executing [0-9a-f]+ (re)
  task-extends-exclude:build: 
  task-extends-exclude:build: > build
  task-extends-exclude:build: > echo running-build
  task-extends-exclude:build: 
  task-extends-exclude:build: running-build
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  

Running test should work (inherited normally)
  $ ${TURBO} run test --filter=task-extends-exclude
  \xe2\x80\xa2 Packages in scope: task-extends-exclude (esc)
  \xe2\x80\xa2 Running test in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  task-extends-exclude:test: cache miss, executing [0-9a-f]+ (re)
  task-extends-exclude:test: 
  task-extends-exclude:test: > test
  task-extends-exclude:test: > echo running-test
  task-extends-exclude:test: 
  task-extends-exclude:test: running-test
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  

Running lint should fail because it was excluded via extends: false
  $ ${TURBO} run lint --filter=task-extends-exclude
  \xe2\x80\xa2 Packages in scope: task-extends-exclude (esc)
  \xe2\x80\xa2 Running lint in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s+[.0-9]+m?s  (re)
  

