Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd) basic_monorepo "npm@8.19.4"

Run test run
  $ ${TURBO} run build --__test-run | jq .package_manager
  "npm"

Set package manager to yarn in package.json
  $ jq '.packageManager = "yarn@1.22.7"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ ${TURBO} run build --__test-run | jq .package_manager
  "yarn"

Set up .yarnrc.yml
  $ echo "nodeLinker: node-modules" > .yarnrc.yml

Set package manager to berry in package.json
  $ jq '.packageManager = "yarn@2.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ ${TURBO} run build --__test-run | jq .package_manager
  "berry"

Set package manager to pnpm6 in package.json
  $ jq '.packageManager = "pnpm@6.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ ${TURBO} run build --__test-run | jq .package_manager
  "pnpm6"

Set package manager to pnpm in package.json
  $ jq '.packageManager = "pnpm@7.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ ${TURBO} run build --__test-run | jq .package_manager
  "pnpm"
