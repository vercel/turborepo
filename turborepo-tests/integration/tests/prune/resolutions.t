Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../../../helpers/copy_fixture.sh $(pwd) berry_resolutions ${TESTDIR}/../../fixtures
  $ export TURBO_GLOBAL_WARNING_DISABLED=1

Prune a
We expect both the original and resolved descriptors to be preserved
when there are yarn resolutions
  $ ${TURBO} prune a
  Generating pruned monorepo for a in .*out (re)
   - Added a
  $ grep -F '"is-odd@' out/yarn.lock
  "is-odd@^0.1.2, is-odd@npm:3.0.0":
    resolution: "is-odd@npm:3.0.0"


Prune b
We should no longer have the override for is-odd
  $ ${TURBO} prune b
  Generating pruned monorepo for b in .*out (re)
   - Added b


  $ grep -F '"is-odd@npm:' out/yarn.lock
  "is-odd@npm:^3.0.1":
    resolution: "is-odd@npm:3.0.1"
