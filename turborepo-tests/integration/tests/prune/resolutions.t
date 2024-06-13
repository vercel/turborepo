Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../../../helpers/copy_fixture.sh $(pwd) berry_resolutions ${TESTDIR}/../../fixtures
  $ export TURBO_GLOBAL_WARNING_DISABLED=1

Prune a
We expect to no longer have the non-resolved is-odd descriptor and
only have the override that has been set
  $ ${TURBO} prune a
  Generating pruned monorepo for a in .*out (re)
   - Added a
  $ grep -F '"is-odd@npm:' out/yarn.lock
  "is-odd@npm:3.0.0":
    resolution: "is-odd@npm:3.0.0"


Prune b
We should no longer have the override for is-odd
  $ ${TURBO} prune b
  Generating pruned monorepo for b in .*out (re)
   - Added b


  $ grep -F '"is-odd@npm:' out/yarn.lock
  "is-odd@npm:^3.0.1":
    resolution: "is-odd@npm:3.0.1"
