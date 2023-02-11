Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# Put some bad JSON into the turbo.json in this app
  $ echo '{"pipeline": {"trailing-comma": {},}}' > "$TARGET_DIR/apps/bad-json/turbo.json"
# The test is greping from a logfile because the list of errors can appear in any order

Errors are shown if we run a task that is misconfigured (invalid-config#build)
  $ ${TURBO} run trailing-comma --filter=bad-json > tmp.log 2>&1
  [1]
  $ cat tmp.log
   something went wrong
