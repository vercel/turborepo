Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Run build and record a trace
Ignore output since we want to focus on testing the generated profile
  $ ${TURBO} build --profile=build.trace > turbo.log
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
Make sure the resulting trace is valid JSON
  $ node -e "require('./build.trace')"
