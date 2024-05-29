Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

  $ rm -rf .turbo/runs

Use --filter because otherwise we'll get nondeterministic execution summary depending on
whether the other workspaces had finished executing their task yet. We don't care to validate
that behavior in this test.
  $ ${TURBO} run maybefails --filter=my-app --summarize > /dev/null 2>&1
  [1]

  $ source "$TESTDIR/../../../helpers/run_summary.sh"
  $ SUMMARY=$(/bin/ls .turbo/runs/*.json | head -n1)

Validate that there was a failed task and exitCode is 1 (which is what we get from npm for the failed task)
  $ cat $SUMMARY | jq '.execution'
  {
    "command": "turbo run maybefails --filter=my-app",
    "repoPath": "",
    "success": 0,
    "failed": 1,
    "cached": 0,
    "attempted": 1,
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "exitCode": 1
  }

Validate that we got a full task summary for the failed task with an error in .execution
  $ echo $(getSummaryTaskId $SUMMARY "my-app#maybefails") | jq
  {
    "taskId": "my-app#maybefails",
    "task": "maybefails",
    "package": "my-app",
    "hash": "9f05a7188fdf4e93",
    "inputs": {
      ".env.local": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
      "package.json": "1746e0db2361085b5953a6a3beab08c24af5bc08"
    },
    "hashOfExternalDependencies": "459c029558afe716",
    "cache": {
      "local": false,
      "remote": false,
      "status": "MISS",
      "timeSaved": 0
    },
    "command": "exit 4",
    "cliArguments": [],
    "outputs": null,
    "excludedOutputs": null,
    "logFile": "apps(\/|\\\\)my-app(\/|\\\\).turbo(\/|\\\\)turbo-maybefails\.log", (re)
    "directory": "apps(\/|\\\\)my-app", (re)
    "dependencies": [],
    "dependents": [],
    "resolvedTaskDefinition": {
      "outputs": [],
      "cache": true,
      "dependsOn": [],
      "inputs": [],
      "outputLogs": "full",
      "persistent": false,
      "env": [],
      "passThroughEnv": null,
      "interactive": false
    },
    "expandedOutputs": [],
    "framework": "",
    "envMode": "strict",
    "environmentVariables": {
      "specified": {
        "env": [],
        "passThroughEnv": null
      },
      "configured": [],
      "inferred": [],
      "passthrough": null
    },
    "execution": {
      "startTime": [0-9]+, (re)
      "endTime": [0-9]+, (re)
      "error": "command .*npm(?:\.cmd)? run maybefails exited \(1\)", (re)
      "exitCode": 1
    }
  }

With --continue flag. We won't validate the whole thing, just execution
Use  --force flag so we can be deterministic about cache hits
Don't use --filter here, so we can validate that both tasks attempted to run
  $ rm -rf .turbo/runs
  $ ${TURBO} run maybefails --summarize --force --continue > /dev/null  2>&1
  [1]

  $ source "$TESTDIR/../../../helpers/run_summary.sh"
  $ SUMMARY=$(/bin/ls .turbo/runs/*.json | head -n1)

success should be 1, and attempted should be 2
  $ cat $SUMMARY | jq '.execution'
  {
    "command": "turbo run maybefails --continue",
    "repoPath": "",
    "success": 1,
    "failed": 1,
    "cached": 0,
    "attempted": 2,
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "exitCode": 1
  }

  $ cat $SUMMARY | jq '.tasks | length'
  2

# exitCode is 1, because npm will report all errors with exitCode 1
  $ getSummaryTaskId $SUMMARY "my-app#maybefails" | jq '.execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "error": "command .*npm(?:\.cmd)? run maybefails exited \(1\)", (re)
    "exitCode": 1
  }
