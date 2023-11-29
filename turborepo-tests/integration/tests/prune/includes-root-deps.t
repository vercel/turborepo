Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh $(pwd) monorepo_with_root_dep

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune web
  Generating pruned monorepo for web in .*(\/|\\)out (re)
   - Added shared
   - Added util
   - Added web
