Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) strict_env_vars

With --env-mode=infer

Baseline global hash
  $ BASELINE=$(${TURBO} build -vv 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")

There's no config to start, so the global hash does not change when flag is passed
  $ WITH_FLAG=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")
  $ test $BASELINE = $WITH_FLAG

Add empty config for global pass through env var, global hash changes
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/global_pt-empty.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ WITH_EMPTY_GLOBAL=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")
  $ test $BASELINE != $WITH_EMPTY_GLOBAL

Add global pass through env var, global hash changes again, because we changed the value
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/global_pt.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ WITH_GLOBAL=$(${TURBO} build -vv --env-mode=infer 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")
  $ test $WITH_EMPTY_GLOBAL != $WITH_GLOBAL
