Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh $(pwd)

  $ ${TURBO} bin -vvv > out.log 2>&1
  $ grep --quiet "Global turbo version: .*" out.log
  $ grep --quiet "No local turbo binary found at" out.log
  $ grep --quiet "Running command as global turbo" out.log
  $ tail -n1 out.log | grep --quiet -E ".*[\/\\]target[\/\\]debug[\/\\]turbo(\.exe)?$"
