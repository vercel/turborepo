Setup
  $ . ${TESTDIR}/../setup.sh

Make sure we use local and don't pass --skip-infer to old binary
  $ . ${TESTDIR}/setup.sh 1.2.3
  $ ${TURBO} build --filter foo
  build --filter foo --single-package --

Make sure we use local and pass --skip-infer to newer binary
  $ . ${TESTDIR}/setup.sh 1.8.9
  $ ${TURBO} build --filter foo
  --skip-infer build --filter foo --single-package --
