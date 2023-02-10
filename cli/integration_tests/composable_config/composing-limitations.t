Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

  $ ${TURBO} run build --filter=package-task
   ERROR  run failed: error preparing engine: Invalid turbo.json in "package-task" workspace. Error: Detected "package-task#build", use "build" instead
  Turbo error: error preparing engine: Invalid turbo.json in "package-task" workspace. Error: Detected "package-task#build", use "build" instead
  [1]

Same error even if you're running a valid task in the package.
  $ ${TURBO} run valid-task --filter=package-task
   ERROR  run failed: error preparing engine: Invalid turbo.json in "package-task" workspace. Error: Detected "package-task#build", use "build" instead
  Turbo error: error preparing engine: Invalid turbo.json in "package-task" workspace. Error: Detected "package-task#build", use "build" instead
  [1]
