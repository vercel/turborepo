Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package_deps

  $ ${TURBO} run test --dry=json
  {
    "id": "[a-zA-Z0-9]+", (re)
    "version": "1",
    "turboVersion": "[a-z0-9\.-]+", (re)
    "monorepo": false,
    "globalCacheInputs": {
      "rootKey": "HEY STELLLLLLLAAAAAAAAAAAAA",
      "files": {
        "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
        "package.json": "bc24e5c5b8bd13d419e0742ae3e92a2bf61c53d0"
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
        "hash": "cb6df6cef2cfd596",
        "inputs": {
          ".gitignore": "6f23ff6842b5526da43ab38f4a5bf3b0158eeb42",
          "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
          "package.json": "bc24e5c5b8bd13d419e0742ae3e92a2bf61c53d0",
          "turbo.json": "e1fe3e5402fe019ef3845cc63a736878a68934c7"
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
        "dependents": [
          "test"
        ],
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
      },
      {
        "taskId": "test",
        "task": "test",
        "hash": "af35177522814c73",
        "inputs": {
          ".gitignore": "6f23ff6842b5526da43ab38f4a5bf3b0158eeb42",
          "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
          "package.json": "bc24e5c5b8bd13d419e0742ae3e92a2bf61c53d0",
          "turbo.json": "e1fe3e5402fe019ef3845cc63a736878a68934c7"
        },
        "hashOfExternalDependencies": "",
        "cache": {
          "local": false,
          "remote": false,
          "status": "MISS",
          "timeSaved": 0
        },
        "command": "[[ ( -f foo ) \u0026\u0026 $(cat foo) == 'building' ]]",
        "cliArguments": [],
        "outputs": null,
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-test.log",
        "dependencies": [
          "build"
        ],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [],
          "cache": true,
          "dependsOn": [
            "build"
          ],
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

  $ ${TURBO} run test --dry=json --experimental-rust-codepath
  {
    "id": "[a-zA-Z0-9]+", (re)
    "version": "1",
    "turboVersion": "[a-z0-9\.-]+", (re)
    "monorepo": false,
    "globalCacheInputs": {
      "rootKey": "HEY STELLLLLLLAAAAAAAAAAAAA",
      "files": {
        "package-lock.json": "1c117cce37347befafe3a9cba1b8a609b3600021",
        "package.json": "bc24e5c5b8bd13d419e0742ae3e92a2bf61c53d0"
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
    "tasks": [],
    "user": ".*", (re)
    "scm": {
      "type": "git",
      "sha": "[a-z0-9]+", (re)
      "branch": ".+" (re)
    }
  }
 }