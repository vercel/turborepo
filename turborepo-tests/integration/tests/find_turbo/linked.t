Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "linked"

Make sure we use local, but do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.0.0"

# Save output to file so we can look for specific things
  $ ${TURBO} build --filter foo -vv > out.log

  $ cat out.log | grep "Repository Root" # Repo root is correct
  .* .*/linked.t (re)

  $ cat out.log | grep "Running local turbo binary in" # The local linked binary is running
  .* .*/linked.t/node_modules/.pnpm/turbo-(darwin|linux|windows)-(64|arm64)@1.0.0/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)

last log line is the script execution. our stubbed turbo binary reflects back the args passed to it
  $ cat out.log | tail -n1
  build --filter foo -vv --

Make sure we use local, and DO pass --skip-infer to newer binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.8.0"

# Save output to file so we can look for specific things
  $ ${TURBO} build --filter foo -vv > out.log

# Repo root is correct
  $ cat out.log | grep "Repository Root"
  .* .*/linked.t (re)

# The local linked binary is running
  $ cat out.log | grep "Running local turbo binary in"
  .* .*/linked.t/node_modules/.pnpm/turbo-(darwin|linux|windows)-(64|arm64)@1.0.0/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)

last log line is the script execution. our stubbed turbo binary reflects back the args passed to it
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --
