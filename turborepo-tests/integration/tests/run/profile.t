Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Run build and record a trace
Ignore output since we want to focus on testing the generated profile
  $ ${TURBO} build --profile=build.trace > turbo.log
Make sure the resulting trace is valid JSON
  $ node -e "require('./build.trace')"
