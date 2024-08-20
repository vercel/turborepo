Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Create a new branch
  $ git checkout -b my-branch
  Switched to a new branch 'my-branch'

Edit a file that affects `my-app`
  $ echo "foo" >> apps/my-app/index.js

Validate that we only run `my-app#build` with change not committed
  $ ${TURBO} run build --affected --log-order grouped
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
  

Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  1 package (npm)
  
    my-app apps[\/\\]my-app (re)


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { name } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": [
        {
          "name": "my-app"
        }
      ]
    }
  }

Commit the change
  $ git add .
  $ git commit -m "add foo" --quiet

Validate that we only run `my-app#build` with change committed
  $ ${TURBO} run build --affected --log-order grouped
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs 97b34acb6e848096
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  

Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  1 package (npm)
  
    my-app apps[\/\\]my-app (re)


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { name } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": [
        {
          "name": "my-app"
        }
      ]
    }
  }

Override the SCM base to be HEAD, so nothing runs
  $ TURBO_SCM_BASE="HEAD" ${TURBO} run build --affected --log-order grouped
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  

Do the same thing with the `ls` command
  $ TURBO_SCM_BASE="HEAD" ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  0 no packages (npm)
  


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages(base: \"HEAD\") { name } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": []
    }
  }

Override the SCM head to be main, so nothing runs
  $ TURBO_SCM_HEAD="main" ${TURBO} run build --affected --log-order grouped
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  

Do the same thing with the `ls` command
  $ TURBO_SCM_HEAD="main" ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  0 no packages (npm)
  


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages(head: \"main\") { name } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": []
    }
  }

Now add a commit to `main` so the merge base is different from `main`
  $ git checkout main --quiet
  $ echo "foo" >> packages/util/index.js
  $ git add .
  $ git commit -m "add foo" --quiet
  $ git checkout my-branch --quiet

Run the build and expect only `my-app` to be affected, since between
`git merge-base main my-branch` and `my-branch` that is the only changed package.
  $ ${TURBO} run build --affected --log-order grouped
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs 97b34acb6e848096
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  1 package (npm)
  
    my-app apps[\/\\]my-app (re)


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { name } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": [
        {
          "name": "my-app"
        }
      ]
    }
  }

Now do some magic to change the repo to be shallow
  $ SHALLOW=$(git rev-parse --show-toplevel)/.git/shallow
  $ git rev-parse HEAD > "$SHALLOW"
  $ git reflog expire --expire=0
  $ git prune
  $ git prune-packed

Now try running `--affected` again, we should run all tasks
  $ ${TURBO} run build --affected --log-order grouped
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: main...HEAD: no merge base
  
  \xe2\x80\xa2 Packages in scope: //, another, my-app, util (esc)
  \xe2\x80\xa2 Running build in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs 97b34acb6e848096
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  util:build: cache miss, executing bf1798d3e46e1b48
  util:build: 
  util:build: > build
  util:build: > echo building
  util:build: 
  util:build: building
  
   Tasks:    2 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: main...HEAD: no merge base
  
  3 packages (npm)
  
    another packages[\/\\]another (re)
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { name } }"
   WARNING  query command is experimental and may change in the future
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: main...HEAD: no merge base
  
  {
    "data": {
      "affectedPackages": [
        {
          "name": "//"
        },
        {
          "name": "another"
        },
        {
          "name": "my-app"
        },
        {
          "name": "util"
        }
      ]
    }
  }