Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) pnpm

Add new package with an external dependency
  $ mkdir -p apps/c
  $ echo '{"name":"c", "dependencies": {"has-symbols": "^1.0.3"}}' > apps/c/package.json

Update lockfile
  $ pnpm i --frozen-lockfile=false > /dev/null

Now build and verify that only the new package is in scope
Note that we need --skip-infer because we've now installed a local
turbo in this repo
Note that we need to disable path conversion because on windows, git bash considers
'//' to be an escape sequence translating to '/'.
  $ MSYS_NO_PATHCONV=1 ${TURBO} --skip-infer build -F '[HEAD]' -F '!//' --dry=json | jq '.packages' 
  [
    "c"
  ]


