Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

We write into a file because prysk doesn't play well
with the square brackets miette uses for source file paths
  $ ${TURBO} something > tmp.log 2>&1
  [1]
  $ grep --quiet -E "root task //#something \(turbo run build\) looks like it invokes turbo and" tmp.log
  $ grep --quiet -E "might cause a loop" tmp.log
  $ grep --quiet -E "task found here" tmp.log
  $ grep --quiet -E "\"something\": \"turbo run build\"" tmp.log


