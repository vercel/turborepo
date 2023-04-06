Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) monorepo

With strict mode

Get Baseline global hash
  $ BASELINE=$(${TURBO} build -vv 2>&1 | "$TESTDIR/./get-global-hash.sh")

Hash changes, because we're using a new mode
  $ WITH_FLAG=$(${TURBO} build -vv --experimental-env-mode=strict 2>&1 | "$TESTDIR/./get-global-hash.sh")
  $ test $BASELINE != $WITH_FLAG

Add empty config for global pass through env var
Hash does not change, because the mode is the same and we haven't added any new pass through vars
  $ cp "$TESTDIR/fixture-configs/global_pt-empty.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ WITH_EMPTY_GLOBAL=$(${TURBO} build -vv --experimental-env-mode=strict 2>&1 | "$TESTDIR/./get-global-hash.sh")
  $ test $WITH_FLAG = $WITH_EMPTY_GLOBAL

Add global pass through env var
Hash changes, because we have a new pass through value
  $ cp "$TESTDIR/fixture-configs/global_pt.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ WITH_GLOBAL=$(${TURBO} build -vv --experimental-env-mode=strict 2>&1 | "$TESTDIR/./get-global-hash.sh")
  $ test $WITH_EMPTY_GLOBAL != $WITH_GLOBAL
