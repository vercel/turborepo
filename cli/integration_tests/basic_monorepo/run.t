Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

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
