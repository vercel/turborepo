Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh single_package

  $ ${TURBO} run build --dry=json
  {
    "id": "[a-zA-Z0-9]+", (re)
    "version": "1",
    "turboVersion": "[a-z0-9\.-]+", (re)
    "monorepo": false,
    "globalCacheInputs": {
      "rootKey": "I can\xe2\x80\x99t see ya, but I know you\xe2\x80\x99re here", (esc)
      "files": {
        "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
        "package.json": "8606ff4b95a5330740d8d9d0948faeada64f1f32",
        "somefile.txt": "45b983be36b73c0788dc9cbcb76cbb80fc7bb057"
      },
      "hashOfExternalDependencies": "",
      "hashOfInternalDependencies": "",
      "environmentVariables": {
        "specified": {
          "env": [],
          "passThroughEnv": null
        },
        "configured": [],
        "inferred": [],
        "passthrough": null
      },
      "engines": null
    },
    "envMode": "strict",
    "frameworkInference": true,
    "tasks": [
      {
        "taskId": "build",
        "task": "build",
        "hash": "7ece7b62aad25615",
        "inputs": {
          ".gitignore": "03b541460c1b836f96f9c0a941ceb48e91a9fd83",
          "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
          "package.json": "8606ff4b95a5330740d8d9d0948faeada64f1f32",
          "somefile.txt": "45b983be36b73c0788dc9cbcb76cbb80fc7bb057",
          "turbo.json": "ce5bdbed55601768de641f5d8d005a8f5be8d3f7"
        },
        "hashOfExternalDependencies": "",
        "cache": {
          "local": false,
          "remote": false,
          "status": "MISS",
          "timeSaved": 0
        },
        "command": "echo building > foo.txt",
        "cliArguments": [],
        "outputs": [
          "foo.txt"
        ],
        "excludedOutputs": null,
        "logFile": ".turbo(\/|\\\\)turbo-build.log", (re)
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [
            "foo.txt"
          ],
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
        }
      }
    ],
    "user": ".*", (re)
    "scm": {
      "type": "git",
      "sha": "[a-z0-9]+", (re)
      "branch": ".+" (re)
    }
  }
  
