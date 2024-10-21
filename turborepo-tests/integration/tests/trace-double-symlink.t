Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh oxc_repro

  $ ${TURBO} query "query { file(path: \"./index.js\") { path dependencies { files { items { path } } errors { items { message import } } } } }"
