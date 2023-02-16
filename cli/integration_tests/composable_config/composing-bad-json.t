Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# Put some bad JSON into the turbo.json in this app
  $ echo '{"pipeline": {"trailing-comma": {},}}' > "$TARGET_DIR/apps/bad-json/turbo.json"
# The test is greping from a logfile because the list of errors can appear in any order

# Errors are shown if we run across a malformed turbo.json
  $ ${TURBO} run trailing-comma --filter=bad-json > tmp.log 2>&1
  [1]
  $ cat tmp.log
   ERROR  run failed: error preparing engine: turbo.json: invalid character '}' looking for beginning of object key string
  Turbo error: error preparing engine: turbo.json: invalid character '}' looking for beginning of object key string
