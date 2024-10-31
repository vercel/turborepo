Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Create a new branch
  $ git checkout -b my-branch
  Switched to a new branch 'my-branch'

Ensure that nothing is affected
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  0 no packages (npm)
  

Create a new file that affects `my-app`
  $ echo "foo" > apps/my-app/new.js

Validate that we only run `my-app#build` with change not committed
  $ ${TURBO} run build --affected --log-order grouped
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing 1b83c3b24476ec9c
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   WARNING  no output files found for task my-app#build. Please check your `outputs` key in `turbo.json`


Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  1 package (npm)
  
    my-app apps[\/\\]my-app (re)



Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { items { name reason { __typename } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": {
        "items": [
          {
            "name": "my-app",
            "reason": {
              "__typename": "FileChanged"
            }
          }
        ]
      }
    }
  }

Also with `affectedFiles` in `turbo query`
  $ ${TURBO} query "query { affectedFiles { items { path, affectedPackages { items { name } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedFiles": {
        "items": [
          {
            "path": "apps(\/|\\\\)my-app(\/|\\\\)new.js", (re)
            "affectedPackages": {
              "items": [
                {
                  "name": "my-app"
                }
              ]
            }
          }
        ]
      }
    }
  }

Remove the new file
  $ rm apps/my-app/new.js

Add a file in `util`
  $ echo "hello world" > packages/util/new.js

Validate that both `my-app` and `util` are affected
  $ ${TURBO} query "query { affectedPackages { items { name reason { __typename } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": {
        "items": [
          {
            "name": "my-app",
            "reason": {
              "__typename": "DependencyChanged"
            }
          },
          {
            "name": "util",
            "reason": {
              "__typename": "FileChanged"
            }
          }
        ]
      }
    }
  }

Remove the new file
  $ rm packages/util/new.js

Add field to `apps/my-app/package.json`
  $ jq '. += {"description": "foo"}' apps/my-app/package.json | tr -d '\r' > apps/my-app/package.json.new
  $ mv apps/my-app/package.json.new apps/my-app/package.json

Validate that we only run `my-app#build` with change not committed
  $ ${TURBO} run build --affected --log-order grouped
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing c1189254892f813f
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   WARNING  no output files found for task my-app#build. Please check your `outputs` key in `turbo.json`

Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
  1 package (npm)
  
    my-app apps[\/\\]my-app (re)


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { items { name reason { __typename } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": {
        "items": [
          {
            "name": "my-app",
            "reason": {
              "__typename": "FileChanged"
            }
          }
        ]
      }
    }
  }

Also with `affectedFiles` in `turbo query`
  $ ${TURBO} query "query { affectedFiles { items { path, affectedPackages { items { name } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedFiles": {
        "items": [
          {
            "path": "apps(\/|\\\\)my-app(\/|\\\\)package.json", (re)
            "affectedPackages": {
              "items": [
                {
                  "name": "my-app"
                }
              ]
            }
          }
        ]
      }
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
  my-app:build: cache hit, replaying logs c1189254892f813f
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
  $ ${TURBO} query "query { affectedPackages { items { name } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": {
        "items": [
          {
            "name": "my-app"
          }
        ]
      }
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
  $ ${TURBO} query "query { affectedPackages(base: \"HEAD\") { items { name } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": {
        "items": []
      }
    }
  }

Also with `affectedFiles` in `turbo query`
  $ ${TURBO} query "query { affectedFiles(base: \"HEAD\") { items { path, affectedPackages { items { name } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedFiles": {
        "items": []
      }
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
  $ ${TURBO} query "query { affectedPackages(head: \"main\") { items { name } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": {
        "items": []
      }
    }
  }

Also with `affectedFiles` in `turbo query`
  $ ${TURBO} query "query { affectedFiles(head: \"main\") { items { path, affectedPackages { items { name } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedFiles": {
        "items": []
      }
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
  my-app:build: cache hit, replaying logs c1189254892f813f
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
  $ ${TURBO} query "query { affectedPackages { items { name } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedPackages": {
        "items": [
          {
            "name": "my-app"
          }
        ]
      }
    }
  }

Also with `affectedFiles` in `turbo query`
  $ ${TURBO} query "query { affectedFiles { items { path, affectedPackages { items { name } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "affectedFiles": {
        "items": [
          {
            "path": "apps(\/|\\\\)my-app(\/|\\\\)package.json", (re)
            "affectedPackages": {
              "items": [
                {
                  "name": "my-app"
                }
              ]
            }
          }
        ]
      }
    }
  }

Now do some magic to change the repo to be shallow
  $ SHALLOW=$(git rev-parse --show-toplevel)/.git/shallow
  $ git rev-parse HEAD > "$SHALLOW"
  $ git reflog expire --expire=0
  $ git prune
  $ git prune-packed

Now try running `--affected` again, we should run all tasks
  $ ${TURBO} run build --affected --dry-run json | jq '.tasks | map(.taskId)| sort'
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: no merge base found
  
  [
    "another#build",
    "my-app#build",
    "util#build"
  ]

Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: no merge base found
  
  3 packages (npm)
  
    another packages[\/\\]another (re)
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { items { name } } }"
   WARNING  query command is experimental and may change in the future
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: no merge base found
  
  {
    "data": {
      "affectedPackages": {
        "items": [
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
  }

Now do some magic to change the repo to be shallow
  $ SHALLOW=$(git rev-parse --show-toplevel)/.git/shallow
  $ git rev-parse HEAD > "$SHALLOW"
  $ git reflog expire --expire=0
  $ git prune
  $ git prune-packed

Now try running `--affected` again, we should run all tasks
  $ ${TURBO} run build --affected --dry-run json | jq '.tasks | map(.taskId)| sort'
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: no merge base found
  
  [
    "another#build",
    "my-app#build",
    "util#build"
  ]

Do the same thing with the `ls` command
  $ ${TURBO} ls --affected
   WARNING  ls command is experimental and may change in the future
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: no merge base found
  
  3 packages (npm)
  
    another packages[\/\\]another (re)
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)


Do the same thing with the `query` command
  $ ${TURBO} query "query { affectedPackages { items { name } } }"
   WARNING  query command is experimental and may change in the future
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: no merge base found
  
  {
    "data": {
      "affectedPackages": {
        "items": [
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
  }

Use a filter with `affectedPackages`
  $ ${TURBO} query "query { affectedPackages(filter: { equal: { field: NAME, value: \"my-app\" } }) { items { name } } }"
   WARNING  query command is experimental and may change in the future
   WARNING  unable to detect git range, assuming all files have changed: git error: fatal: no merge base found
  
  {
    "data": {
      "affectedPackages": {
        "items": [
          {
            "name": "my-app"
          }
        ]
      }
    }
  }

