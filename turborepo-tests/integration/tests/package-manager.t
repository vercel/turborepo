Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh basic_monorepo "npm@8.19.4"

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "npm"

Set package manager to yarn in package.json
  $ jq '.packageManager = "yarn@1.22.7"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "yarn"

Set up .yarnrc.yml
  $ echo "nodeLinker: node-modules" > .yarnrc.yml

Set package manager to berry in package.json
  $ jq '.packageManager = "yarn@2.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "berry"

Set package manager to pnpm6 in package.json
  $ jq '.packageManager = "pnpm@6.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Set up pnpm-workspace.yaml
  $ echo "packages:" >> pnpm-workspace.yaml
  $ echo "  - apps/*" >> pnpm-workspace.yaml

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "pnpm6"

Set package manager to pnpm in package.json
  $ jq '.packageManager = "pnpm@7.0.0"' package.json > package.json.tmp && mv package.json.tmp package.json

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "pnpm"

Clear package manager field in package.json
  $ jq 'del(.packageManager)' package.json > package.json.tmp && mv package.json.tmp package.json

Delete package-lock.json
  $ rm package-lock.json

Use yarn 1.22.19
  $ corepack prepare yarn@1.22.19 --activate
  Preparing yarn@1.22.19 for immediate activation...

Create yarn.lock
  $ touch yarn.lock

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "yarn"

Use yarn 3.5.1
  $ corepack prepare yarn@3.5.1 --activate
  Preparing yarn@3.5.1 for immediate activation...

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "berry"

Delete yarn.lock
  $ rm yarn.lock

Create pnpm-lock.yaml
  $ touch pnpm-lock.yaml

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "pnpm"

Delete pnpm-lock.yaml
  $ rm pnpm-lock.yaml

Create package-lock.json
  $ touch package-lock.json

Run test run
  $ TURBO_LOG_VERBOSITY=off ${TURBO} info --json | jq .packageManager
  "npm"
