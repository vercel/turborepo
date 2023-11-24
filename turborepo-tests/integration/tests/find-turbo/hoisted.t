Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "hoisted"

Make sure we use local and do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.0.0"
  $ ${TURBO} build --filter foo -vv > out.log 2>&1
  $ grep --quiet -F "Local turbo version: 1.0.0" out.log
  $ cat out.log | tail -n1
  build --filter foo -vv --

Make sure we use local and pass --skip-infer to newer binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.8.0"
  $ ${TURBO} build --filter foo -vv > out.log 2>&1
  $ grep --quiet -F "Local turbo version: 1.8.0" out.log
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --
