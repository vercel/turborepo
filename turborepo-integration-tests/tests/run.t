Setup
  $ . ${TESTDIR}/_helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

# Running non-existent tasks errors
  $ ${TURBO} run doesnotexist
   ERROR  run failed: error preparing engine: Could not find the following tasks in project: doesnotexist
  Turbo error: error preparing engine: Could not find the following tasks in project: doesnotexist
  [1]

# Multiple non-existent tasks also error
  $ ${TURBO} run doesnotexist alsono
   ERROR  run failed: error preparing engine: Could not find the following tasks in project: alsono, doesnotexist
  Turbo error: error preparing engine: Could not find the following tasks in project: alsono, doesnotexist
  [1]

# One good and one bad task does not error
  $ ${TURBO} run build doesnotexist
   ERROR  run failed: error preparing engine: Could not find the following tasks in project: doesnotexist
  Turbo error: error preparing engine: Could not find the following tasks in project: doesnotexist
  [1]

# Bad command
  $ ${TURBO} run something --dry
  root task something (turbo run build) looks like it invokes turbo and might cause a loop
   ERROR  run failed: errors occurred during dry-run graph traversal
  Turbo error: errors occurred during dry-run graph traversal
  [1]

# Bad command
  $ ${TURBO} run something
  \xe2\x80\xa2 Packages in scope: //, another, my-app, util (esc)
  \xe2\x80\xa2 Running something in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  root task something (turbo run build) looks like it invokes turbo and might cause a loop
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]
