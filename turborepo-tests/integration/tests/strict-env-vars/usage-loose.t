Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh strict_env_vars

With --env-mode=loose, all vars are available

Set the env vars
  $ export GLOBAL_VAR_PT=higlobalpt
  $ export GLOBAL_VAR_DEP=higlobaldep
  $ export LOCAL_VAR_PT=hilocalpt
  $ export LOCAL_VAR_DEP=hilocaldep
  $ export OTHER_VAR=hiother
  $ export SYSTEMROOT=hisysroot
Set the output file with the right path separator for the OS
  $ if [[ "$OSTYPE" == "msys" ]]; then OUTPUT="apps\\my-app\\out.txt"; else OUTPUT="apps/my-app/out.txt"; fi

All vars available in loose mode
  $ ${TURBO} build -vv --env-mode=loose > /dev/null 2>&1
  $ cat "$OUTPUT"
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: 'hiother', sysroot set: 'yes', path set: 'yes'

All vars available in loose mode, even when global and pass through configs defined
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $(pwd) "strict_env_vars/all.json"
  $ ${TURBO} build -vv --env-mode=loose > /dev/null 2>&1
  $ cat "$OUTPUT"
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: 'hiother', sysroot set: 'yes', path set: 'yes'
