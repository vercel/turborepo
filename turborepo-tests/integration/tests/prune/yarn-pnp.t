Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../../../helpers/copy_fixture.sh $(pwd) berry_resolutions ${TESTDIR}/../../fixtures
Remove linker override
  $ rm .yarnrc.yml
Prune the project
  $ ${TURBO} prune a
  Generating pruned monorepo for a in .*out (re)
   - Added a

Verify that .pnp.cjs isn't coppied causing unnecessary cache misses
  $ ls -A out/
  package.json
  packages
  yarn.lock
