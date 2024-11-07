Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/10-too-many

Validate that we get an error when we try to run multiple persistent tasks with concurrency 1
  $ ${TURBO} run build --concurrency=1
    x invalid task configuration
  
  Error:   x You have 2 persistent tasks but `turbo` is configured for concurrency of
    | 1. Set --concurrency to at least 3
  
  [1]

However on query, we ignore this validation
  $ ${TURBO} query "query { packages { items { tasks { items { fullName } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "tasks": {
              "items": []
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "one#build"
                }
              ]
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "two#build"
                }
              ]
            }
          }
        ]
      }
    }
  }

Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/invalid-dependency
  warning: re-init: ignored --initial-branch=main

Validate that we get an error when trying to depend on a task that doesn't exist
  $ ${TURBO} run build2
    x Could not find "app-a#custom" in root turbo.json or "custom" in package
      ,-[turbo.json:27:1]
   27 |       "dependsOn": [
   28 |         "app-a#custom"
      :         ^^^^^^^^^^^^^^
   29 |       ]
      `----
  
  [1]

However, we don't get an error when we query
  $ ${TURBO} query "query { packages { items { tasks { items { fullName } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "tasks": {
              "items": []
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "app-a#build"
                },
                {
                  "fullName": "app-a#test"
                }
              ]
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "app-b#build"
                },
                {
                  "fullName": "app-b#test"
                }
              ]
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "lib-a#build"
                },
                {
                  "fullName": "lib-a#test"
                }
              ]
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "lib-b#build"
                },
                {
                  "fullName": "lib-b#test"
                }
              ]
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "lib-c#build"
                }
              ]
            }
          },
          {
            "tasks": {
              "items": [
                {
                  "fullName": "lib-d#build"
                },
                {
                  "fullName": "lib-d#test"
                }
              ]
            }
          }
        ]
      }
    }
  }