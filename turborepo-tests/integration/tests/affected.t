Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Create a new branch
  $ git checkout -b my-branch
  Switched to a new branch 'my-branch'

Edit and commit a file that affects `my-app`
  $ echo "foo" >> apps/my-app/index.js
  $ git add .
  $ git commit -m "add foo" --quiet

Validate that we only run `my-app#build`
  $ ${TURBO} run build --affected
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing 97b34acb6e848096
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  


Now do some magic to change the repo to be shallow
  $ SHALLOW=$(git rev-parse --show-toplevel)/.git/shallow
  $ git rev-parse HEAD > "$SHALLOW"
  $ git reflog expire --expire=0
  $ git prune
  $ git prune-packed

Now try running `--affected` again, we should run all tasks
  $ ${TURBO} run build --affected
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: main...HEAD: no merge base
  
  \xe2\x80\xa2 Packages in scope: //, another, my-app, util (esc)
  \xe2\x80\xa2 Running build in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing bf1798d3e46e1b48
  my-app:build: cache hit, replaying logs 97b34acb6e848096
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  util:build: 
  util:build: > build
  util:build: > echo building
  util:build: 
  util:build: building
  
   Tasks:    2 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  


