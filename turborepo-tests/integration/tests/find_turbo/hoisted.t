Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "hoisted"

Make sure we use local and do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.0.0"

  $ echo ${ROOT_DIR}
  $ echo ${TURBO}

  $ ${TURBO} bin -vvv

  $ echo "pwd: $PWD"
  $ echo "$PWD/node_modules/turbo-windows-64/bin/turbo.exe"
  $ cat -vet "$PWD/node_modules/turbo-windows-64/bin/turbo.exe"
  $ cat -vet "$PWD/node_modules/turbo-windows-arm64/bin/turbo.exe"
  
  $ echo "PRYSK_TEMP: $PRYSK_TEMP"

  $ . "$PRYSK_TMP/hoisted.t/node_modules/turbo-windows-64/bin/turbo.exe" hi yes # direct call from prysk tmp
  
  $ "$PWD/node_modules/turbo-windows-64/bin/turbo.exe" hi yes # direct call from pwd

  $ ${TURBO} build --filter foo -vv # debug version

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
