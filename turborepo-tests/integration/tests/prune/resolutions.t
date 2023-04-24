Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/copy_fixture.sh $(pwd) berry_resolutions

Prune a
We expect to no longer have the non-resolved is-odd descriptor and
only have the override that has been set
  $ ${TURBO} prune --scope=a
  Generating pruned monorepo for a in .*out (re)
   - Added a
  $ grep -F '"is-odd@npm:' out/yarn.lock
  "is-odd@npm:3.0.0":
    resolution: "is-odd@npm:3.0.0"


Prune b
We should no longer have the override for is-odd
  $ ${TURBO} prune --scope=b
  Generating pruned monorepo for b in .*out (re)
   - Added b


  $ grep -F '"is-odd@npm:' out/yarn.lock
  "is-odd@npm:^3.0.1":
    resolution: "is-odd@npm:3.0.1"
