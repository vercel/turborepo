Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

  $ ${TURBO} run build --filter=package-task
   ERROR  run failed: error preparing engine: Detected "package-task#build" in "package-task". Declare "build" instead
  Turbo error: error preparing engine: Detected "package-task#build" in "package-task". Declare "build" instead
  [1]
