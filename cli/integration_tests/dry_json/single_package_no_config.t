Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package_no_config

  $ ${TURBO} run build --dry=json
  {
    "id": "[a-zA-Z0-9]+", (re)
    "version": "0",
    "turboVersion": "[a-z0-9\.-]+", (re)
    "globalCacheInputs": {
      "rootKey": "Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo",
      "files": {
        "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
        "package.json": "581fe2b8dcba5b03cbe51d78a973143eb6d33e3a"
      },
      "hashOfExternalDependencies": "",
      "rootPipeline": {
        "//#build": {
          "outputs": [],
          "cache": false,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        }
      }
    },
    "tasks": [
      {
        "taskId": "build",
        "task": "build",
        "hash": "c7223f212c321d3b",
        "inputs": {
          ".gitignore": "38548b0538f2fc563d6bacf70dd42798c6fd9a35",
          "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
          "package.json": "581fe2b8dcba5b03cbe51d78a973143eb6d33e3a"
        },
        "hashOfExternalDependencies": "",
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
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [],
          "cache": false,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        },
        "expandedOutputs": [],
        "framework": "\u003cNO FRAMEWORK DETECTED\u003e",
        "environmentVariables": {
          "configured": [],
          "inferred": [],
          "global": [
            "VERCEL_ANALYTICS_ID="
          ]
        }
      }
    ]
  }
