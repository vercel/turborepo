Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) strict_env_vars

With strict mode

Get Baseline global hash
  $ BASELINE=$(${TURBO} build -vv 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")

Hash changes, because we're using a new mode
  $ WITH_FLAG=$(${TURBO} build -vv --env-mode=strict 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")
  $ test $BASELINE != $WITH_FLAG

Add empty config for global pass through env var
Hash does not change, because the mode is the same and we haven't added any new pass through vars
  $ . ${TESTDIR}/../../../helpers/replace_turbo_config.sh $(pwd) "strict_env_vars/global_pt-empty.json"
  $ WITH_EMPTY_GLOBAL=$(${TURBO} build -vv --env-mode=strict 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")
  $ test $WITH_FLAG = $WITH_EMPTY_GLOBAL

Add global pass through env var
Hash changes, because we have a new pass through value
  $ . ${TESTDIR}/../../../helpers/replace_turbo_config.sh $(pwd) "strict_env_vars/global_pt.json"
  $ WITH_GLOBAL=$(${TURBO} build -vv --env-mode=strict 2>&1 | "$TESTDIR/../_helpers/get-global-hash.sh")
  $ test $WITH_EMPTY_GLOBAL != $WITH_GLOBAL
