Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "hoisted"

Make sure we use local and do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.0.0"
  $ ${TURBO} bin
  $ $PWD/node_modules/.bin/turbo bin
  $ cat -vet node_modules/turbo-windows-64/bin/turbo.exe
  $ cat -vet node_modules/turbo-windows-arm64/bin/turbo.exe
  $ ./node_modules/turbo-windows-arm64/bin/turbo.exe hi yes

  $ ${TURBO} build --filter foo -vv > out.log
  $ grep --quiet -F "Local turbo version: 1.0.0" out.log
  $ cat out.log | tail -n1
  build --filter foo -vv --

Make sure we use local and pass --skip-infer to newer binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.8.0"
  $ ${TURBO} build --filter foo -vv > out.log
  $ grep --quiet -F "Local turbo version: 1.8.0" out.log
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --
