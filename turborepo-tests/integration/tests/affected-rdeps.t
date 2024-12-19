Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Create a new branch
  $ git checkout -b my-branch
  Switched to a new branch 'my-branch'

Edit a file that affects `util` package
  $ echo "foo" >> packages/util/index.js
Commit the change
  $ git add .
  $ git commit -m "add foo" --quiet

Validate that we run `util#build` and all rdeps
  $ ${TURBO} run build --affected --dry=json | jq '.tasks |  map(select(.command != "<NONEXISTENT>")) | map(.taskId)| sort'
  [
    "my-app#build",
    "util#build"
  ]
