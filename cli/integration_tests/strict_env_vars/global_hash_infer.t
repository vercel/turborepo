Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) monorepo

With --experimental-env-mode=infer

Baseline global hash
  $ BASELINE=$(${TURBO} build -vv 2>&1 | "$TESTDIR/./get-global-hash.sh")

There's no config to start, so the global hash does not change when flag is passed
  $ WITH_FLAG=$(${TURBO} build -vv --experimental-env-mode=infer 2>&1 | "$TESTDIR/./get-global-hash.sh")
  $ test $BASELINE = $WITH_FLAG

Add empty config for global pass through env var, global hash changes
  $ cp "$TESTDIR/fixture-configs/global_pt-empty.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ WITH_EMPTY_GLOBAL=$(${TURBO} build -vv --experimental-env-mode=infer 2>&1 | "$TESTDIR/./get-global-hash.sh")
  $ test $BASELINE != $WITH_EMPTY_GLOBAL

Add global pass through env var, global hash changes again, because we changed the value
  $ cp "$TESTDIR/fixture-configs/global_pt.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ WITH_GLOBAL=$(${TURBO} build -vv --experimental-env-mode=infer 2>&1 | "$TESTDIR/./get-global-hash.sh")
  $ test $WITH_EMPTY_GLOBAL != $WITH_GLOBAL
