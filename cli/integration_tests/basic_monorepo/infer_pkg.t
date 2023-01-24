Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)
 
Run a dry run
  $ ${TURBO} build --dry=json | jq .packages
  [
    "my-app",
    "util"
  ]

Run a dry run in a directory
  $ cd packages/util
  $ ${TURBO} build --dry=json | jq .packages
  [
    "util"
  ]

Ensure we don't infer packages if --cwd is supplied
  $ ${TURBO} build --cwd=../.. --dry=json | jq .packages
  [
    "my-app",
    "util"
  ]
