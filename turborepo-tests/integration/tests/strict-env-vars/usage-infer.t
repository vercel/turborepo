Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh strict_env_vars

With --env-mode=infer

Set the env vars
  $ export GLOBAL_VAR_PT=higlobalpt
  $ export GLOBAL_VAR_DEP=higlobaldep
  $ export LOCAL_VAR_PT=hilocalpt
  $ export LOCAL_VAR_DEP=hilocaldep
  $ export OTHER_VAR=hiother

Set the output file with the right path separator for the OS
  $ if [[ "$OSTYPE" == "msys" ]]; then OUTPUT="apps\\my-app\\out.txt"; else OUTPUT="apps/my-app/out.txt"; fi

Conditionally set these vars if they aren't already there for the purpose of the test.
The test doesn't care about the values, it just checks that the var is available to the task
so we just have to make sure the parent process has them set. In Github CI, for example SHELL
isn't already set.
  $ export SYSTEMROOT="${SYSTEMROOT:=hisysroot}"
  $ export PATH="${PATH:=hipath}"

Inferred mode as loose because no pass through configs, all vars are available
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat "$OUTPUT"
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: 'hiother', sysroot set: 'yes', path set: 'yes'

Inferred mode as strict, because global pass through config, no vars available
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $(pwd) "strict_env_vars/global_pt-empty.json"
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat "$OUTPUT"
  globalpt: '', localpt: '', globaldep: '', localdep: '', other: '', sysroot set: 'yes', path set: 'yes'

Inferred mode as strict, because task pass through config, no vars available
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $(pwd) "strict_env_vars/task_pt-empty.json"
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat "$OUTPUT"
  globalpt: '', localpt: '', globaldep: '', localdep: '', other: '', sysroot set: 'yes', path set: 'yes'

Inferred mode as strict, with declared deps and pass through. all declared available, other is not available
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $(pwd) "strict_env_vars/all.json"
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat "$OUTPUT"
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: '', sysroot set: 'yes', path set: 'yes'
