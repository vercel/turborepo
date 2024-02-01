Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Force git status to show a file with spaces in the name
  $ for i in {1..10000}; do echo "new file" > packages/util/with\ spaces\ ${i}.txt; done

Verify we have a file with spaces in the name
  $ git status | grep "with spaces" | wc -l
     10000

Do a dry run to verify we can hash it
  $ ${TURBO} run build --dry -F util | grep "Inputs Files Considered"
    Inputs Files Considered        = 10001
