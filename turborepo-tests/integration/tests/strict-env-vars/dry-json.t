Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh strict_env_vars

Empty passthroughs are null
  $ ${TURBO} build --dry=json | jq -r '.tasks[0].environmentVariables | { passthrough, globalPassthrough }'
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  {
    "passthrough": null,
    "globalPassthrough": null
  }

Make sure that we populate the JSON output
  $ ${TESTDIR}/../../../helpers/replace_turbo_config.sh $(pwd) "strict_env_vars/all.json"
  $ ${TURBO} build --dry=json | jq -r '.tasks[0].environmentVariables | { passthrough, globalPassthrough }'
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  {
    "passthrough": [],
    "globalPassthrough": null
  }
