Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) strict_env_vars

With --experimental-env-mode=strict, only declared vars are available

Set the env vars
  $ export GLOBAL_VAR_PT=higlobalpt
  $ export GLOBAL_VAR_DEP=higlobaldep
  $ export LOCAL_VAR_PT=hilocalpt
  $ export LOCAL_VAR_DEP=hilocaldep
  $ export OTHER_VAR=hiother
  $ export SYSTEMROOT=hisysroot

No vars available by default
  $ ${TURBO} build -vv --experimental-env-mode=strict > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: '', localpt: '', globaldep: '', localdep: '', other: '', sysroot set: 'yes', path set: 'yes'

All declared vars available, others are not available
  $ cp "$TESTDIR/fixture-configs/all.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build -vv --experimental-env-mode=strict > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: '', sysroot set: 'yes', path set: 'yes'
