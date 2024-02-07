Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

Login Test Run
  $ ${TURBO} login --__test-run
  Login test run successful

Login reuses Vercel CLI token
  $ . ${TESTDIR}/../../helpers/mock_existing_login.sh
  $ ${TURBO} login
  Existing Vercel token found!
