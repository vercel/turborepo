Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)
 
Run a dry run
  $ ${TURBO} build --dry=json | jq .packages
  [
    "another",
    "my-app",
    "util"
  ]

Run a dry run in packages with a filter
  $ cd packages
  $ ${TURBO} build --dry=json -F "{./util}" | jq .packages
  [
    "util"
  ]

Run a dry run in a directory
  $ cd util
  $ ${TURBO} build --dry=json | jq .packages
  [
    "util"
  ]

Ensure we don't infer packages if --cwd is supplied
  $ ${TURBO} build --cwd=../.. --dry=json | jq .packages
  [
    "another",
    "my-app",
    "util"
  ]
