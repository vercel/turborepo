Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# The test is greping from a logfile because the list of errors can appear in any order

Errors are shown if we run a task that is misconfigured (invalid-config#build)
  $ ${TURBO} run build --filter=invalid-config > tmp.log 2>&1
  [1]
  $ cat tmp.log | grep "Invalid turbo.json"
   ERROR  run failed: error preparing engine: Invalid turbo.json
  Turbo error: error preparing engine: Invalid turbo.json
  $ cat tmp.log | grep "invalid-config#build"
   - "invalid-config#build". Use "build" instead
   - "invalid-config#build". Use "build" instead
  $ cat tmp.log | grep "//#some-root-task"
   - "//#some-root-task". Use "some-root-task" instead
   - "//#some-root-task". Use "some-root-task" instead
  $ cat tmp.log | grep "extends"
   - No "extends" key found
   - No "extends" key found

Same error even if you're running a valid task in the package.
  $ ${TURBO} run valid-task --filter=invalid-config > tmp.log 2>&1
  [1]
  $ cat tmp.log | grep "Invalid turbo.json"
   ERROR  run failed: error preparing engine: Invalid turbo.json
  Turbo error: error preparing engine: Invalid turbo.json
  $ cat tmp.log | grep "invalid-config#build"
   - "invalid-config#build". Use "build" instead
   - "invalid-config#build". Use "build" instead
  $ cat tmp.log | grep "//#some-root-task"
   - "//#some-root-task". Use "some-root-task" instead
   - "//#some-root-task". Use "some-root-task" instead
  $ cat tmp.log | grep "extends"
   - No "extends" key found
   - No "extends" key found
