Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/../../helpers/mock_turbo_config.sh

Link Test Run
  $ ${TURBO} link --__test-run
  Link test run successful

  $ ${TURBO} link --__test-run --yes
  Link test run successful

  $ ${TURBO} link --__test-run --team=my-team
   WARNING  team flag does not set the scope for linking. Use --scope instead.
  Link test run successful


