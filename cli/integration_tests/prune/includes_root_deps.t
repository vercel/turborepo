Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune --scope=web
  Generating pruned monorepo for web in .*/out (re)
   - Added shared
   - Added util
   - Added web
