Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package                                                                                  

  $ ${TURBO} run build --dry=json
  {
    "id": "[a-zA-Z0-9]+", (re)
    "version": "1",
    "turboVersion": "[a-z0-9\.-]+", (re)
    "monorepo": false,
    "globalCacheInputs": {
      "rootKey": "You don't understand! I coulda had class. I coulda been a contender. I could've been somebody, instead of a bum, which is what I am.",
      "files": {
        "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
        "package.json": "185771929d92c3865ce06c863c07d357500d3364",
        "somefile.txt": "45b983be36b73c0788dc9cbcb76cbb80fc7bb057"
      },
      "hashOfExternalDependencies": "",
      "globalDotEnv": null,
      "environmentVariables": {
        "specified": {
          "env": [],
          "passThroughEnv": null
        },
        "configured": [],
        "inferred": [],
        "passthrough": null
      }
    },
    "envMode": "infer",
    "frameworkInference": true,
    "tasks": [
      {
        "taskId": "build",
        "task": "build",
        "hash": "9d6c858db9fd1eea",
        "inputs": {
          ".gitignore": "6f23ff6842b5526da43ab38f4a5bf3b0158eeb42",
          "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
          "package.json": "185771929d92c3865ce06c863c07d357500d3364",
          "somefile.txt": "45b983be36b73c0788dc9cbcb76cbb80fc7bb057",
          "turbo.json": "505752e75c10f9e7a0d2538cf8b6f0fcfb8980a0"
        },
        "hashOfExternalDependencies": "",
        "cache": {
          "local": false,
          "remote": false,
          "status": "MISS",
          "timeSaved": 0
        },
        "command": "echo 'building' \u003e foo",
        "cliArguments": [],
        "outputs": [
          "foo"
        ],
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [
            "foo"
          ],
          "cache": true,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "persistent": false,
          "env": [],
          "passThroughEnv": null,
          "dotEnv": null
        },
        "expandedOutputs": [],
        "framework": "\u003cNO FRAMEWORK DETECTED\u003e",
        "envMode": "loose",
        "environmentVariables": {
          "specified": {
            "env": [],
            "passThroughEnv": null
          },
          "configured": [],
          "inferred": [],
          "passthrough": null
        },
        "dotEnv": null
      }
    ],
    "user": ".*", (re)
    "scm": {
      "type": "git",
      "sha": "[a-z0-9]+", (re)
      "branch": ".+" (re)
    }
  }
