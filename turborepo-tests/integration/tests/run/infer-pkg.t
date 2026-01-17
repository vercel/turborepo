Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh
 
Run a dry run
  $ ${TURBO} build --dry=json | jq .packages
  [
    "another",
    "my-app",
    "util"
  ]

Run a dry run in packages with a glob filter
  $ ${TURBO} build --dry=json -F "./packages/*" | jq .packages
  [
    "another",
    "util"
  ]

Run a dry run in packages with a name glob
  $ ${TURBO} build --dry=json -F "*-app" | jq .packages
  [
    "my-app"
  ]

Run a dry run in packages with a filter
  $ cd packages
  $ ${TURBO} build --dry=json -F "{./util}" | jq .packages
  [
    "util"
  ]
Run a dry run with a filter from a sibling directory
  $ ${TURBO} build --dry=json -F "../apps/*" | jq .packages
  [
    "my-app"
  ]

Run a dry run with a filter name glob
  $ ${TURBO} build --dry=json -F "*-app" | jq .packages
  [
    "my-app"
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

Run a dry run in packages with a glob filter from directory
  $ ${TURBO} build --dry=json -F "../*" | jq .packages
  [
    "util"
  ]

Run a dry run in packages with a name glob from directory
  $ ${TURBO} build --dry=json -F "*nother" | jq .packages
  [
    "another"
  ]
