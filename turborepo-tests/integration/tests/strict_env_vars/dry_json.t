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
  $ cp "$TESTDIR/../_fixtures/strict_env_vars_configs/all.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks[0].environmentVariables | { passthrough, globalPassthrough }'
  {
    "passthrough": [],
    "globalPassthrough": null
  }
