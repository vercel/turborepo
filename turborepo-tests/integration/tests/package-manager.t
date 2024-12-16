Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh basic_monorepo "npm@8.19.4"

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} config | jq .packageManager
  "npm"

Set package manager to yarn in package.json
  $ jq '.packageManager = "yarn@1.22.7"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} config | jq .packageManager
  "yarn"

Set up .yarnrc.yml
  $ echo "nodeLinker: node-modules" > .yarnrc.yml

Set package manager to berry in package.json
  $ jq '.packageManager = "yarn@2.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} config | jq .packageManager
  "berry"

Set package manager to pnpm6 in package.json
  $ jq '.packageManager = "pnpm@6.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Set up pnpm-workspace.yaml
  $ echo "packages:" >> pnpm-workspace.yaml
  $ echo "  - apps/*" >> pnpm-workspace.yaml

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} config | jq .packageManager
  "pnpm6"

Set package manager to pnpm in package.json
  $ jq '.packageManager = "pnpm@7.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} config | jq .packageManager
  "pnpm"
