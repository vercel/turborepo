Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

# Save JSON to tmp file so we don't need to keep re-running the build
  $ ${TURBO} run build --dry=json > tmpjson.log

  $ cat tmpjson.log | jq .globalHashSummary
  {
    "globalFileHashMap": {
      "foo.txt": "eebae5f3ca7b5831e429e947b7d61edd0de69236"
    },
    "rootExternalDepsHash": "ccab0b28617f1f56",
    "globalCacheKey": "Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo",
    "pipeline": {
      "build": {
        "outputs": [],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [],
        "persistent": false
      },
      "my-app#build": {
        "outputs": [
          "apple.json",
          "banana.txt"
        ],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [],
        "persistent": false
      }
    }
  }

# Validate output of my-app#build task
  $ cat tmpjson.log | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0]'
  {
    "taskId": "my-app#build",
    "task": "build",
    "package": "my-app",
    "hash": "8888a278aaecb070",
    "cacheState": {
      "local": false,
      "remote": false
    },
    "command": "echo 'building'",
    "outputs": [
      "apple.json",
      "banana.txt"
    ],
    "excludedOutputs": null,
    "logFile": "apps/my-app/.turbo/turbo-build.log",
    "directory": "apps/my-app",
    "dependencies": [],
    "dependents": [],
    "resolvedTaskDefinition": {
      "outputs": [
        "apple.json",
        "banana.txt"
      ],
      "cache": true,
      "dependsOn": [],
      "inputs": [],
      "outputMode": "full",
      "env": [],
      "persistent": false
    },
    "expandedInputs": {
      "package.json": "f2a5d2525f3996a57680180a7cd9ad7310e4dec0"
    },
    "framework": "<NO FRAMEWORK DETECTED>"
  }

# Validate output of util#build task
  $ cat tmpjson.log | jq '.tasks | map(select(.taskId == "util#build")) | .[0]'
  {
    "taskId": "util#build",
    "task": "build",
    "package": "util",
    "hash": "d09a52ea72495c87",
    "cacheState": {
      "local": false,
      "remote": false
    },
    "command": "echo 'building'",
    "outputs": null,
    "excludedOutputs": null,
    "logFile": "packages/util/.turbo/turbo-build.log",
    "directory": "packages/util",
    "dependencies": [],
    "dependents": [],
    "resolvedTaskDefinition": {
      "outputs": [],
      "cache": true,
      "dependsOn": [],
      "inputs": [],
      "outputMode": "full",
      "env": [],
      "persistent": false
    },
    "expandedInputs": {
      "package.json": "8d3e121335e16dbd8d99c03522b892ec52416dda"
    },
    "framework": "<NO FRAMEWORK DETECTED>"
  }

Tasks that don't exist throw an error
  $ ${TURBO} run doesnotexist --dry=json
   ERROR  run failed: error preparing engine: Could not find the following tasks in project: doesnotexist
  Turbo error: error preparing engine: Could not find the following tasks in project: doesnotexist
  [1]
