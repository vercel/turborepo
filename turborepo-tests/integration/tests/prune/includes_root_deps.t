Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) monorepo_with_root_dep

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune --scope=web
  Generating pruned monorepo for web in .*/out (re)
   - Added shared
   - Added util
   - Added web
