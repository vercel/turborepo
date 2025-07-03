Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_with_root_dep pnpm@7.25.1

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune web
  Generating pruned monorepo for web in .*(\/|\\)out (re)
   - Added shared
   - Added util
   - Added web

Make sure turbo.jsonc is copied over
  $ mv turbo.json turbo.jsonc
  $ rm -r out
  $ ${TURBO} prune web
  Generating pruned monorepo for web in .*(\/|\\)out (re)
   - Added shared
   - Added util
   - Added web
  $ ls out
  apps
  package.json
  packages
  patches
  pnpm-lock.yaml
  pnpm-workspace.yaml
  turbo.jsonc
