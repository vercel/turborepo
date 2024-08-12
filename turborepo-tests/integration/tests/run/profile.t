Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Run build and record a trace
Ignore output since we want to focus on testing the generated profile
  $ ${TURBO} build --profile=build.trace > turbo.log
  my-app:build: warning: no files were found that match the configured outputs - make sure "outputs" are correctly defined in your `turbo.json` for my-app#build
Make sure the resulting trace is valid JSON
  $ node -e "require('./build.trace')"
