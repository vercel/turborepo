Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

  $ git config uploadpack.allowFilter true
  $ cd ..
Make sure we allow partial clones

Do a blobless clone
  $ ${TURBO} clone file://$(pwd)/basic-monorepo.t basic-monorepo-blobless --local
  Cloning into 'basic-monorepo-blobless'...
  $ cd basic-monorepo-blobless
  $ ${TURBO} build > /dev/null 2>&1
  $ cd ..

Do a treeless clone
  $ ${TURBO} clone file://$(pwd)/basic-monorepo.t basic-monorepo-treeless --local
  Cloning into 'basic-monorepo-treeless'...
  $ cd basic-monorepo-treeless
  $ ${TURBO} build > /dev/null 2>&1