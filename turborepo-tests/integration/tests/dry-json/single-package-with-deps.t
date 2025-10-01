Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh single_package

  $ ${TURBO} run test --dry=json
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
        "hash": "fe0059df5e6291b2",
        "inputs": {
          ".gitignore": "03b541460c1b836f96f9c0a941ceb48e91a9fd83",
          "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
          "package.json": "8606ff4b95a5330740d8d9d0948faeada64f1f32",
          "somefile.txt": "45b983be36b73c0788dc9cbcb76cbb80fc7bb057",
          "turbo.json": "3bc68ed1f2a5a308cb0166f9ed073c2fc7980ac7"
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
        "logFile": "\.turbo(\/|\\\\)turbo-build\.log", (re)
        "dependencies": [],
        "dependents": [
          "test"
        ],
        "with": [],
        "resolvedTaskDefinition": {
          "outputs": [
            "foo.txt"
          ],
          "cache": true,
          "dependsOn": [],
          "inputs": [],
          "outputLogs": "full",
          "persistent": false,
          "interruptible": false,
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
      },
      {
        "taskId": "test",
        "task": "test",
        "hash": "7cfbd8e30495d802",
        "inputs": {
          ".gitignore": "03b541460c1b836f96f9c0a941ceb48e91a9fd83",
          "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
          "package.json": "8606ff4b95a5330740d8d9d0948faeada64f1f32",
          "somefile.txt": "45b983be36b73c0788dc9cbcb76cbb80fc7bb057",
          "turbo.json": "3bc68ed1f2a5a308cb0166f9ed073c2fc7980ac7"
        },
        "hashOfExternalDependencies": "",
        "cache": {
          "local": false,
          "remote": false,
          "status": "MISS",
          "timeSaved": 0
        },
        "command": "cat foo.txt",
        "cliArguments": [],
        "outputs": null,
        "excludedOutputs": null,
        "logFile": "\.turbo(\/|\\\\)turbo-test\.log", (re)
        "dependencies": [
          "build"
        ],
        "dependents": [],
        "with": [],
        "resolvedTaskDefinition": {
          "outputs": [],
          "cache": true,
          "dependsOn": [
            "build"
          ],
          "inputs": [],
          "outputLogs": "full",
          "persistent": false,
          "interruptible": false,
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
  
