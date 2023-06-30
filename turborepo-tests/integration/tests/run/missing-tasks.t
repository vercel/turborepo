Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

# Running non-existent tasks errors
  $ ${TURBO} run doesnotexist
  Error: Could not find the following tasks in project: doesnotexist
  [1]

# Multiple non-existent tasks also error
  $ ${TURBO} run doesnotexist alsono
  Error: Could not find the following tasks in project: alsono, doesnotexist
  [1]

# One good and one bad task does not error
  $ ${TURBO} run build doesnotexist
  Error: Could not find the following tasks in project: doesnotexist
  [1]

# Bad command
  $ ${TURBO} run something --dry
  Error: root task //#something (turbo run build) looks like it invokes turbo and might cause a loop
  [1]

# Bad command
  $ ${TURBO} run something
  Error: root task //#something (turbo run build) looks like it invokes turbo and might cause a loop
  [1]
