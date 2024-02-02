Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh basic_monorepo
  $ pushd ${TESTDIR}/../../../../packages/turbo-repository >> /dev/null
  $ pnpm build >> /dev/null 2>&1
  $ popd >> /dev/null
  $ npm install ${TESTDIR}/../../../../packages/turbo-repository/js >> /dev/null 2>&1
  $ git commit --quiet -am "Add @turbo/repository script" >> /dev/null
  $ cp ${TESTDIR}/script.mjs $PWD
  $ node script.mjs | jq 'keys | sort'
  [
    "apps/my-app",
    "packages/another",
    "packages/util"
  ]
