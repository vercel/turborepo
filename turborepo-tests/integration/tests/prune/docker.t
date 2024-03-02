Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_with_root_dep pnpm@7.25.1

  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web
Make sure patches are part of the json output
  $ ls out/json
  apps
  package.json
  packages
  patches
  pnpm-lock.yaml
  pnpm-workspace.yaml
Make sure patches are part of the json output
  $ ls out/full
  apps
  package.json
  packages
  patches
  pnpm-workspace.yaml
  turbo.json
Make sure that pnpm-workspace.yaml is in the top out directory
  $ ls out
  full
  json
  pnpm-lock.yaml
  pnpm-workspace.yaml

Make sure the pnpm patches section is present
  $ cat out/json/package.json | jq '.pnpm.patchedDependencies'
  {
    "is-number@7.0.0": "patches/is-number@7.0.0.patch"
  }
