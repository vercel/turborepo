Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Running non-existent tasks errors
  $ ${TURBO} run doesnotexist
    x missing tasks in project
  
  Error:   x could not find task `doesnotexist` in project
  
  [1]

# Multiple non-existent tasks also error
  $ ${TURBO} run doesnotexist alsono
    x missing tasks in project
  
  Error:   x could not find task `alsono` in project
  Error:   x could not find task `doesnotexist` in project
  
  [1]

# One good and one bad task does not error
  $ ${TURBO} run build doesnotexist
    x missing tasks in project
  
  Error:   x could not find task `doesnotexist` in project
  
  [1]

# Bad command
  $ ${TURBO} run something --dry > OUTPUT 2>&1
  [1]
  $ grep --quiet -E "root task (//#)?something \(turbo run build\) looks like it invokes turbo and" OUTPUT
  $ grep --quiet -E "might cause a loop" OUTPUT

# Bad command

  $ ${TURBO} run something > OUTPUT2 2>&1
  [1]
  $ grep --quiet -E "root task (//#)?something \(turbo run build\) looks like it invokes turbo and" OUTPUT
  $ grep --quiet -E "might cause a loop" OUTPUT
