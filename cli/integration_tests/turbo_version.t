Setup
  $ . ${TESTDIR}/setup.sh

Test version matches that of version.txt
  $ FROM_FILE=$(head -n 1 ${VERSION})
  $ FROM_CLI="${TURBO} --version"
  $ bash -c 'diff <(${FROM_FILE}) <(${FROM_CLI})'

TODO: resolve ambiguity
$ ${TURBO} -v
