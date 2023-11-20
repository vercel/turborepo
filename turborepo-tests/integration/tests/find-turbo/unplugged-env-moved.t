Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "unplugged_env_moved"

Make sure we use local and do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.0.0"
  $ set -o allexport; source .env; set +o allexport;
  $ ${TURBO} build --filter foo -vv > out.log 2>&1
  $ grep --quiet -F "Local turbo version: 1.0.0" out.log
  $ grep --quiet -E "Running local turbo binary in .*[\/\\]unplugged-env-moved\.t[\/\\]\.moved[\/\\]unplugged[\/\\].*[\/\\]bin[\/\\]turbo(\.exe)?" out.log
  $ cat out.log | tail -n1
  build --filter foo -vv --

Make sure we use local and pass --skip-infer to newer binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.8.0"
  $ set -o allexport; source .env; set +o allexport;
  $ ${TURBO} build --filter foo -vv > out.log 2>&1
  $ grep --quiet -F "Local turbo version: 1.8.0" out.log
  $ grep --quiet -E "Running local turbo binary in .*[\/\\]unplugged-env-moved\.t[\/\\]\.moved[\/\\]unplugged[\/\\].*[\/\\]bin[\/\\]turbo(\.exe)?" out.log
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --
