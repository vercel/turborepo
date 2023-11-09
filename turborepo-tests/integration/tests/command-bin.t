Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

Run info
  $ ${TURBO} bin -vvv
  $ ${TURBO} bin -vvv > out.log
  $ grep --quiet -E "Global turbo version: .*" out.log
  $ grep --quiet -E "No local turbo binary found at" out.log
  $ grep --quiet -E "Running command as global turbo" out.log
  $ grep --quiet -E ".*/target/debug/turbo" out.log
