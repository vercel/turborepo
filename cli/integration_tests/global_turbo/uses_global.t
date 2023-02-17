Setup
  $ . ${TESTDIR}/../setup.sh

When --skip-infer is used we use the current binary and output no global/local message
  $ ${TURBO} --skip-infer --help | head -n 1
  The build system that makes ship happen
