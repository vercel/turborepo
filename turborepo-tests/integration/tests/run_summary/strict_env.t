Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) strict_env_vars

Set the env vars
  $ export GLOBAL_VAR_PT=higlobalpt
  $ export GLOBAL_VAR_DEP=higlobaldep
  $ export LOCAL_VAR_PT=hilocalpt
  $ export LOCAL_VAR_DEP=hilocaldep
  $ export OTHER_VAR=hiother
  $ export SYSTEMROOT=hisysroot

Run as `infer`
  $ rm -rf .turbo/runs
  $ ${TURBO} run build --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  infer
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": null,
    "globalPassthrough": null
  }

Run as `strict`
  $ rm -rf .turbo/runs
  $ ${TURBO} run build --env-mode=strict --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": null,
    "globalPassthrough": null
  }

Run as `loose`
  $ rm -rf .turbo/runs
  $ ${TURBO} run build --env-mode=loose --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": null,
    "globalPassthrough": null
  }

All specified + infer
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/all.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": [
      "LOCAL_VAR_PT=7cd1bb19c058cf4d6ad6aa579d685bddddf3ab587b78bdcb1e6e488fb6f47a3b"
    ],
    "globalPassthrough": [
      "GLOBAL_VAR_PT=cecd31fff1e723588eac8fe1edff89a6d2ec072f5c3bd884a98297487670b1f0"
    ]
  }

All specified + loose
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/all.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --env-mode=loose --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": [
      "LOCAL_VAR_PT=7cd1bb19c058cf4d6ad6aa579d685bddddf3ab587b78bdcb1e6e488fb6f47a3b"
    ],
    "globalPassthrough": [
      "GLOBAL_VAR_PT=cecd31fff1e723588eac8fe1edff89a6d2ec072f5c3bd884a98297487670b1f0"
    ]
  }

Global passthrough specified empty array + infer
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/global_pt-empty.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": null,
    "globalPassthrough": []
  }

Global passthrough specified value + infer
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/global_pt.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": null,
    "globalPassthrough": [
      "GLOBAL_VAR_PT=cecd31fff1e723588eac8fe1edff89a6d2ec072f5c3bd884a98297487670b1f0"
    ]
  }

Global passthrough specified empty array + loose
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/global_pt-empty.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --env-mode=loose --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": null,
    "globalPassthrough": []
  }

Global passthrough specified value + loose
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/global_pt.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --env-mode=loose --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": null,
    "globalPassthrough": [
      "GLOBAL_VAR_PT=cecd31fff1e723588eac8fe1edff89a6d2ec072f5c3bd884a98297487670b1f0"
    ]
  }

Task passthrough specified empty array + infer
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/task_pt-empty.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  infer
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": [],
    "globalPassthrough": null
  }

Task passthrough specified value + infer
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/task_pt.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  infer
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  strict
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": [
      "LOCAL_VAR_PT=7cd1bb19c058cf4d6ad6aa579d685bddddf3ab587b78bdcb1e6e488fb6f47a3b"
    ],
    "globalPassthrough": null
  }

Task passthrough specified empty array + loose
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/task_pt-empty.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --env-mode=loose --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": [],
    "globalPassthrough": null
  }

Task passthrough specified value + loose
  $ rm -rf .turbo/runs
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/task_pt.json" "$(pwd)/turbo.json" && git commit --allow-empty -am "no comment" --quiet
  $ ${TURBO} run build --env-mode=loose --summarize > /dev/null
  $ cat .turbo/runs/*.json | jq -r '.envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].envMode'
  loose
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables | {passthrough, globalPassthrough}'
  {
    "passthrough": [
      "LOCAL_VAR_PT=7cd1bb19c058cf4d6ad6aa579d685bddddf3ab587b78bdcb1e6e488fb6f47a3b"
    ],
    "globalPassthrough": null
  }
