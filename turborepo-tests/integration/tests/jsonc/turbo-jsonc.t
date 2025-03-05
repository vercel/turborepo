Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Test 1: Error when both turbo.json and turbo.jsonc exist in the same directory
Create both turbo.json and turbo.jsonc in the root
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.json turbo.json
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc

Run turbo build with both files present
  $ ${TURBO} build 2> error.txt
  [1]
  $ cat error.txt
    x Found both turbo.json and turbo.jsonc in the same directory: .* (re)
    | Remove either turbo.json or turbo.jsonc so there is only one.
  

# Test 2: Using turbo.jsonc in the root
Remove turbo.json and use only turbo.jsonc
  $ rm turbo.json
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc

Run turbo build with only turbo.jsonc
  $ ${TURBO} build --output-logs=none
  • Packages in scope: another, my-app, util
  • Running build in 3 packages
  • Remote caching disabled
  
  Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
  Time:    *ms >>> FULL TURBO (re)
  

# Test 3: Using turbo.json in the root and turbo.jsonc in a package
Setup turbo.json in root and turbo.jsonc in a package
  $ rm turbo.jsonc
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.json turbo.json
  $ mkdir -p apps/my-app
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic-with-extends.jsonc apps/my-app/turbo.jsonc

Run turbo build with root turbo.json and package turbo.jsonc
  $ ${TURBO} build --output-logs=none
  • Packages in scope: another, my-app, util (esc)
  • Running build in 3 packages (esc)
  • Remote caching disabled
  
  Tasks:    2 successful, 2 total
  Cached:    1 cached, 2 total
  Time:    *ms (re)
  
  WARNING  no output files found for task my-app#build. Please check your `outputs` key in `turbo.json` (re)
  

# Test 4: Using turbo.jsonc in the root and turbo.json in a package
Setup turbo.jsonc in root and turbo.json in a package
  $ rm turbo.json
  $ rm apps/my-app/turbo.jsonc
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic-with-extends.json apps/my-app/turbo.json

Run turbo build with root turbo.jsonc and package turbo.json
  $ ${TURBO} build --output-logs=none
  • Packages in scope: another, my-app, util (esc)
  • Running build in 3 packages (esc)
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Cached:    1 cached, 2 total
  Time: *s (re)
  

# Test 5: Error when both turbo.json and turbo.jsonc exist in a package
Setup both turbo.json and turbo.jsonc in a package
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/package-task.jsonc apps/my-app/turbo.jsonc

Run turbo build with both files in a package
  $ ${TURBO} build 2> error.txt
  [1]
  $ cat error.txt
    x Found both turbo.json and turbo.jsonc in the same directory: .*/apps/my-app (re)
    | Remove either turbo.json or turbo.jsonc so there is only one.
  
