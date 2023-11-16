Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

# Running non-existent tasks errors
  $ ${TURBO} run doesnotexist
  (Error:| ERROR  run failed:) Could not find the following tasks in project: doesnotexist (re)
  [1]

# Multiple non-existent tasks also error
  $ ${TURBO} run doesnotexist alsono
  (Error:| ERROR  run failed:) Could not find the following tasks in project: alsono, doesnotexist (re)
  [1]

# One good and one bad task does not error
  $ ${TURBO} run build doesnotexist
  (Error:| ERROR  run failed:) Could not find the following tasks in project: doesnotexist (re)
  [1]

# Bad command
  $ ${TURBO} run something --dry 2>&1 | grep --quiet "root task //#something (turbo run build) looks like it invokes turbo and might cause a loop"

# Bad command
  $ ${TURBO} run something 2>&1 | grep --quiet "root task //#something (turbo run build) looks like it invokes turbo and might cause a loop"

