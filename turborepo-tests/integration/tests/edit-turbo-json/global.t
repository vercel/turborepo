Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline global hash
  $ cp "$TESTDIR/fixture-configs/1-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ BASELINE=$(${TURBO} build -vv 2>&1 | "$TESTDIR/../../../helpers/find_global_hash.sh")

Update pipeline: global hash remains stable.
  $ cp "$TESTDIR/fixture-configs/2-update-pipeline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ UPDATE_PIPELINE=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../../../helpers/find_global_hash.sh")
  $ test $BASELINE = $UPDATE_PIPELINE

Update globalEnv: global hash changes.
  $ cp "$TESTDIR/fixture-configs/3-update-global-env.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ NEW_GLOBAL_ENV=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../../../helpers/find_global_hash.sh")
  $ test $BASELINE != $NEW_GLOBAL_ENV

Update globalDeps in a non-material way: global hash remains stable.
  $ cp "$TESTDIR/fixture-configs/4-update-global-deps.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ NEW_GLOBAL_DEPS=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../../../helpers/find_global_hash.sh")
  $ test $BASELINE = $NEW_GLOBAL_DEPS

Update globalDeps in a material way: global hash changes.
  $ cp "$TESTDIR/fixture-configs/5-update-global-deps-materially.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ NEW_GLOBAL_DEPS=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../../../helpers/find_global_hash.sh")
  $ test $BASELINE != $NEW_GLOBAL_DEPS

Update passThroughEnv: global hash changes.
  $ cp "$TESTDIR/fixture-configs/6-update-passthrough-env.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ NEW_GLOBAL_PASSTHROUGH=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../../../helpers/find_global_hash.sh")
  $ test $BASELINE != $NEW_GLOBAL_PASSTHROUGH
