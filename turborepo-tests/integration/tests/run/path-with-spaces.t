Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Force git status to show a file with spaces in the name
  $ echo "new file" > packages/util/with\ spaces.txt

Verify we have a file with spaces in the name
  $ git status | grep -q "with spaces"

Do a dry run to verify we can hash it
  $ ${TURBO} run build --dry -F util | grep "Inputs Files Considered"
    Inputs Files Considered        = 2
