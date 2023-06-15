Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) monorepo_with_root_dep
  $ npm ...
  Unknown command: "..."
  
  To see a list of supported npm commands, run:
    npm help
  [1]

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune --scope=web
  Generating pruned monorepo for web in .*/out (re)
   - Added shared
   - Added util
   - Added web
