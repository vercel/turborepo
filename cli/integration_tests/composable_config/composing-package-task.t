Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

  $ ${TURBO} run build --filter=package-task
   ERROR  run failed: error preparing engine: Invalid turbo.json
   - "package-task#build". Use "build" instead
   - "//#some-root-task". Use "some-root-task" instead
   - No "extends" key found
  Turbo error: error preparing engine: Invalid turbo.json
   - "package-task#build". Use "build" instead
   - "//#some-root-task". Use "some-root-task" instead
   - No "extends" key found
  [1]

Same error even if you're running a valid task in the package.
  $ ${TURBO} run valid-task --filter=package-task
   ERROR  run failed: error preparing engine: Invalid turbo.json
   - "package-task#build". Use "build" instead
   - "//#some-root-task". Use "some-root-task" instead
   - No "extends" key found
  Turbo error: error preparing engine: Invalid turbo.json
   - "package-task#build". Use "build" instead
   - "//#some-root-task". Use "some-root-task" instead
   - No "extends" key found
  [1]
