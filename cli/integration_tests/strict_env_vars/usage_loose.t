Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) monorepo

With --experimental-env-mode=loose, all vars are available

Set the env vars
  $ export GLOBAL_VAR_PT=higlobalpt
  $ export GLOBAL_VAR_DEP=higlobaldep
  $ export LOCAL_VAR_PT=hilocalpt
  $ export LOCAL_VAR_DEP=hilocaldep
  $ export OTHER_VAR=hiother
  $ export SYSTEMROOT=hisysroot

All vars available in loose mode
  $ ${TURBO} build -vv --experimental-env-mode=loose > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: 'hiother', sysroot set: 'yes', path set: 'yes', shell set: 'yes'

All vars available in loose mode, even when global and pass through configs defined
  $ cp "$TESTDIR/fixture-configs/all.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build -vv --experimental-env-mode=loose > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: 'hiother', sysroot set: 'yes', path set: 'yes', shell set: 'yes'
