Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

Test version matches that of version.txt
  $ diff <(head -n 1 ${VERSION}) <(${TURBO} --version)
  1c1
  < 1.10.2-canary.1
  ---
  > 1.10.2-canary.0
  [1]

TODO: resolve ambiguity
$ ${TURBO} -v
