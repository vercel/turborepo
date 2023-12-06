Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_with_root_dep pnpm@7.25.1

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune web
  Generating pruned monorepo for web in .*(\/|\\)out (re)
   - Added shared
   - Added util
   - Added web
