Setup
  $ . ${TESTDIR}/setup.sh

Test version matches that of version.txt
  $ diff <(head -n 1 ${VERSION}) <(${TURBO} --version)
  Repository inference failed: Unable to find `turbo.json` or `package.json` in current path
  Running command as global turbo

TODO: resolve ambiguity
$ ${TURBO} -v
