Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) strict_env_vars

With --env-mode=infer

Set the env vars
  $ export GLOBAL_VAR_PT=higlobalpt
  $ export GLOBAL_VAR_DEP=higlobaldep
  $ export LOCAL_VAR_PT=hilocalpt
  $ export LOCAL_VAR_DEP=hilocaldep
  $ export OTHER_VAR=hiother

Conditionally set these vars if they aren't already there for the purpose of the test.
The test doesn't care about the values, it just checks that the var is available to the task
so we just have to make sure the parent process has them set. In Github CI, for example SHELL
isn't already set.
  $ export SYSTEMROOT="${SYSTEMROOT:=hisysroot}"
  $ export PATH="${PATH:=hipath}"

Inferred mode as loose because no pass through configs, all vars are available
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: 'hiother', sysroot set: 'yes', path set: 'yes'

Inferred mode as strict, because global pass through config, no vars available
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/global_pt-empty.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: '', localpt: '', globaldep: '', localdep: '', other: '', sysroot set: 'yes', path set: 'yes'

Inferred mode as strict, because task pass through config, no vars available
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/task_pt-empty.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: '', localpt: '', globaldep: '', localdep: '', other: '', sysroot set: 'yes', path set: 'yes'

Inferred mode as strict, with declared deps and pass through. all declared available, other is not available
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/all.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build -vv --env-mode=infer > /dev/null 2>&1
  $ cat apps/my-app/out.txt
  globalpt: 'higlobalpt', localpt: 'hilocalpt', globaldep: 'higlobaldep', localdep: 'hilocaldep', other: '', sysroot set: 'yes', path set: 'yes'
