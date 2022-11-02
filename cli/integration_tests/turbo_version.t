Setup
  $ . ${TESTDIR}/setup.sh

Test version matches that of version.txt
  $ diff <(head -n 1 ${VERSION}) <(${TURBO} --version)

  $ diff <(head -n 1 ${VERSION}) <(${SHIM} --version)

TODO: resolve ambiguity
$ ${TURBO} -v
