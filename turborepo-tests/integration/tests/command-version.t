Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

Test version matches that of version.txt
  $ diff <(head -n 1 ${VERSION}) <(${TURBO} --version)
  /bin/sh: line 4: syntax error near unexpected token `('
  /bin/sh: line 4: `diff <(head -n 1 ${VERSION}) <(${TURBO} --version)' (no-eol)

TODO: resolve ambiguity
$ ${TURBO} -v
