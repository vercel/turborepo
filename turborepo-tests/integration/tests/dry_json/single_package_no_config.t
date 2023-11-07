Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package
  $ rm turbo.json
  $ git commit -am "Delete turbo config" --quiet

  $ ${TURBO} run build --dry=json
  {
    "id": "[a-zA-Z0-9]+", (re)
    "version": "1",
    "turboVersion": "[a-z0-9\.-]+", (re)
    "monorepo": false,
    "globalCacheInputs": {
      "rootKey": "HEY STELLLLLLLAAAAAAAAAAAAA",
      "files": {
        "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
        "package.json": "c38e07c8d8d5f851dec8cdbc3d103deed82bc960"
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
        "hash": "db9ab850083a69be",
        "inputs": {
          ".gitignore": "03b541460c1b836f96f9c0a941ceb48e91a9fd83",
          "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
          "package.json": "c38e07c8d8d5f851dec8cdbc3d103deed82bc960",
          "somefile.txt": "45b983be36b73c0788dc9cbcb76cbb80fc7bb057"
        },
        "hashOfExternalDependencies": "",
        "cache": {
          "local": false,
          "remote": false,
          "status": "MISS",
          "timeSaved": 0
        },
        "command": "echo 'building' > foo.txt",
        "cliArguments": [],
        "outputs": null,
        "excludedOutputs": null,
        "logFile": "\.turbo(\/|\\\\)turbo-build\.log", (re)
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [],
          "cache": false,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "persistent": false,
          "env": [],
          "passThroughEnv": null,
          "dotEnv": null
        },
        "expandedOutputs": [],
        "framework": "<NO FRAMEWORK DETECTED>",
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
  
