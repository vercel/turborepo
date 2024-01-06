Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Running non-existent tasks errors
  $ ${TURBO} run doesnotexist
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
   ERROR  run failed: Could not find the following tasks in project: doesnotexist
  [1]

# Multiple non-existent tasks also error
  $ ${TURBO} run doesnotexist alsono
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
   ERROR  run failed: Could not find the following tasks in project: alsono, doesnotexist
  [1]

# One good and one bad task does not error
  $ ${TURBO} run build doesnotexist
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
   ERROR  run failed: Could not find the following tasks in project: doesnotexist
  [1]

# Bad command
  $ ${TURBO} run something --dry 2>&1 | grep --quiet -E "root task (//#)?something \(turbo run build\) looks like it invokes turbo and might cause a loop"

# Bad command
  $ ${TURBO} run something 2>&1 | grep --quiet -E "root task (//#)?something \(turbo run build\) looks like it invokes turbo and might cause a loop"

