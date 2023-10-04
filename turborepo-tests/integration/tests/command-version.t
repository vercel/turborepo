Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

Test version matches that of version.txt
  $ diff <(head -n 1 ${VERSION}) <(${TURBO} --version)


TODO: resolve ambiguity
  $ ${TURBO} -v
  Turbo error: No command specified
  [1]
