Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# The test is greping from a logfile because the list of errors can appear in any order

Errors are shown if we run a task that is misconfigured (invalid-config#build)
  $ ${TURBO} run build --filter=invalid-config > tmp.log 2>&1
  [1]
  $ cat tmp.log | grep --only-matching "invalid turbo.json"
  invalid turbo json
  invalid turbo json
  $ cat tmp.log | grep "invalid-config#build"
    x "invalid-config#build". Use "build" instead
   3 | ,->     "invalid-config#build": {
  $ cat tmp.log | grep "//#some-root-task"
    x "//#some-root-task". Use "some-root-task" instead
   6 |     "//#some-root-task": {},
   6 |         "//#some-root-task": {},
  $ cat tmp.log | grep "extends"
  Error:   x No "extends" key found in apps/invalid-config/turbo.json

Same error even if you're running a valid task in the package.
  $ ${TURBO} run valid-task --filter=invalid-config > tmp.log 2>&1
  [1]
  $ cat tmp.log | grep --only-matching "invalid turbo.json"
  invalid turbo json
  invalid turbo json
  $ cat tmp.log | grep "invalid-config#build"
    x "invalid-config#build". Use "build" instead
   3 | ,->     "invalid-config#build": {
  $ cat tmp.log | grep "//#some-root-task"
    x "//#some-root-task". Use "some-root-task" instead
   6 |     "//#some-root-task": {},
   6 |         "//#some-root-task": {},
  $ cat tmp.log | grep "extends"
  Error:   x No "extends" key found in apps/invalid-config/turbo.json
