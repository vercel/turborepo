Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

# Save JSON to tmp file so we don't need to keep re-running the build
  $ ${TURBO} run build --dry=json > tmpjson.log

# test with a regex that captures what release we usually have (1.x.y or 1.a.b-canary.c)
  $ cat tmpjson.log | jq .turboVersion
  "[a-z0-9\.-]+" (re)

  $ cat tmpjson.log | jq .globalCacheInputs
  {
    "rootKey": "Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo",
    "files": {
      "foo.txt": "eebae5f3ca7b5831e429e947b7d61edd0de69236"
    },
    "hashOfExternalDependencies": "ccab0b28617f1f56",
    "rootPipeline": {
      "//#something": {
        "outputs": [],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [],
        "persistent": false
      },
      "build": {
        "outputs": [],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [
          "NODE_ENV"
        ],
        "persistent": false
      },
      "maybefails": {
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
      },
      "something": {
        "outputs": [],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [],
        "persistent": false
      }
    }
  }

  $ cat tmpjson.log | jq 'keys'
  [
    "envMode",
    "globalCacheInputs",
    "id",
    "packages",
    "scm",
    "tasks",
    "turboVersion",
    "version"
  ]

# Validate output of my-app#build task
  $ cat tmpjson.log | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0]'
  {
    "taskId": "my-app#build",
    "task": "build",
    "package": "my-app",
    "hash": "2f192ed93e20f940",
    "inputs": {
      "package.json": "6bcf57fd6ff30d1a6f40ad8d8d08e8b940fc7e3b"
    },
    "hashOfExternalDependencies": "ccab0b28617f1f56",
    "cache": {
      "local": false,
      "remote": false,
      "status": "MISS",
      "timeSaved": 0
    },
    "command": "echo 'building'",
    "cliArguments": [],
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
    "expandedOutputs": [],
    "framework": "<NO FRAMEWORK DETECTED>",
    "envMode": "loose",
    "environmentVariables": {
      "configured": [],
      "inferred": [],
      "global": [
        "SOME_ENV_VAR=",
        "VERCEL_ANALYTICS_ID="
      ],
      "passthrough": null,
      "globalPassthrough": null
    }
  }

# Validate output of util#build task
  $ cat tmpjson.log | jq '.tasks | map(select(.taskId == "util#build")) | .[0]'
  {
    "taskId": "util#build",
    "task": "build",
    "package": "util",
    "hash": "af2ba2d52192ee45",
    "inputs": {
      "package.json": "4d57bb28c9967640d812981198a743b3188f713e"
    },
    "hashOfExternalDependencies": "ccab0b28617f1f56",
    "cache": {
      "local": false,
      "remote": false,
      "status": "MISS",
      "timeSaved": 0
    },
    "command": "echo 'building'",
    "cliArguments": [],
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
      "env": [
        "NODE_ENV"
      ],
      "persistent": false
    },
    "expandedOutputs": [],
    "framework": "<NO FRAMEWORK DETECTED>",
    "envMode": "loose",
    "environmentVariables": {
      "configured": [
        "NODE_ENV="
      ],
      "inferred": [],
      "global": [
        "SOME_ENV_VAR=",
        "VERCEL_ANALYTICS_ID="
      ],
      "passthrough": null,
      "globalPassthrough": null
    }
  }

Run again with NODE_ENV set and see the value in the summary. --filter=util workspace so the output is smaller
  $ NODE_ENV=banana ${TURBO} run build --dry=json --filter=util | jq '.tasks | map(select(.taskId == "util#build")) | .[0].environmentVariables'
  {
    "configured": [
      "NODE_ENV=b493d48364afe44d11c0165cf470a4164d1e2609911ef998be868d46ade3de4e"
    ],
    "inferred": [],
    "global": [
      "SOME_ENV_VAR=",
      "VERCEL_ANALYTICS_ID="
    ],
    "passthrough": null,
    "globalPassthrough": null
  }

Tasks that don't exist throw an error
  $ ${TURBO} run doesnotexist --dry=json
   ERROR  run failed: error preparing engine: Could not find the following tasks in project: doesnotexist
  Turbo error: error preparing engine: Could not find the following tasks in project: doesnotexist
  [1]
