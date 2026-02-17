Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Run build and record a trace
Ignore output since we want to focus on testing the generated profile
  $ ${TURBO} build --profile=build.trace > turbo.log
   WARNING  no output files found for task my-app#build. Please check your `outputs` key in `turbo.json`
Make sure the resulting trace is valid JSON
  $ node -e "require('./build.trace')"

Make sure the markdown profile summary was generated
  $ test -f build.trace.md
  $ head -1 build.trace.md
  # CPU Profile
  $ grep -c "Hot Functions" build.trace.md
  1
  $ grep -c "Call Tree" build.trace.md
  1
