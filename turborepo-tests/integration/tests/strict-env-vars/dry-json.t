Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) strict_env_vars

Empty passthroughs are null
  $ ${TURBO} build --dry=json | jq -r '.tasks[0].environmentVariables | { passthrough, globalPassthrough }'
  {
    "passthrough": null,
    "globalPassthrough": null
  }

Make sure that we populate the JSON output
  $ . ${TESTDIR}/../../../helpers/replace_turbo_config.sh $(pwd) "strict_env_vars/all.json"
  $ ${TURBO} build --dry=json | jq -r '.tasks[0].environmentVariables | { passthrough, globalPassthrough }'
  {
    "passthrough": [],
    "globalPassthrough": null
  }
