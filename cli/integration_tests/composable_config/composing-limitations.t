Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

  $ ${TURBO} run build --filter=package-task
   ERROR  run failed: error preparing engine: turbo.json in "package-task" is invalid. Error: Detected "package-task#build", use "build" instead
  Turbo error: error preparing engine: turbo.json in "package-task" is invalid. Error: Detected "package-task#build", use "build" instead
  [1]
