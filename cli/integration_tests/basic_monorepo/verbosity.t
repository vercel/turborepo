Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

$ Verbosity level 1
  $ ${TURBO} build -v --force
  $ ${TURBO} build --verbosity=1 --force

$ Verbosity level 2
  $ ${TURBO} build -vv --force
  $ ${TURBO} build --verbosity=2 --force

$ Verbosity level 3
  $ ${TURBO} build -vvv --force
  $ ${TURBO} build --verbosity=3 --force
