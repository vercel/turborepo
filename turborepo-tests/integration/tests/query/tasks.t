Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/topological

  $ ${TURBO} query "query { package(name: \"my-app\") { tasks { items { name } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "name": "build"
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"my-app\") { tasks { items { name directDependencies { items { name package { name } } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "name": "build",
              "directDependencies": {
                "items": [
                  {
                    "name": "build",
                    "package": {
                      "name": "util"
                    }
                  }
                ]
              }
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"util\") { tasks { items { name directDependents { items { name package { name } } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "name": "build",
              "directDependents": {
                "items": [
                  {
                    "name": "build",
                    "package": {
                      "name": "my-app"
                    }
                  }
                ]
              }
            }
          ]
        }
      }
    }
  }
