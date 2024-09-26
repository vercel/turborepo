Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Query packages
  $ ${TURBO} query "query { packages { items { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "name": "//"
          },
          {
            "name": "another"
          },
          {
            "name": "my-app"
          },
          {
            "name": "util"
          }
        ]
      }
    }
  }

Query packages with equals filter
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { items { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "name": "my-app"
          }
        ]
      }
    }
  }

Query packages that have at least one dependent package
  $ ${TURBO} query "query { packages(filter: { greaterThan: { field: DIRECT_DEPENDENT_COUNT, value: 0 } }) { items { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "name": "util"
          }
        ]
      }
    }
  }

Query packages that have a task named `build`
  $ ${TURBO} query "query { packages(filter: { has: { field: TASK_NAME, value: \"build\" } }) { items { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "name": "my-app"
          },
          {
            "name": "util"
          }
        ]
      }
    }
  }

Query packages that have a task named `build` or `dev`
  $ ${TURBO} query "query { packages(filter: { or: [{ has: { field: TASK_NAME, value: \"build\" } }, { has: { field: TASK_NAME, value: \"dev\" } }] }) { items { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "name": "another"
          },
          {
            "name": "my-app"
          },
          {
            "name": "util"
          }
        ]
      }
    }
  }

Get dependents of `util`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"util\" } }) { items { directDependents { items { name } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "directDependents": {
              "items": [
                {
                  "name": "my-app"
                }
              ]
            }
          }
        ]
      }
    }
  }

Get dependencies of `my-app`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { items { directDependencies { items { name } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "directDependencies": {
              "items": [
                {
                  "name": "util"
                }
              ]
            }
          }
        ]
      }
    }
  }

Get the indirect dependencies of `my-app`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { items { indirectDependencies { items { name } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "indirectDependencies": {
              "items": [
                {
                  "name": "//"
                }
              ]
            }
          }
        ]
      }
    }
  }

Get all dependencies of `my-app`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { items { allDependencies { items { name } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "allDependencies": {
              "items": [
                {
                  "name": "//"
                },
                {
                  "name": "util"
                }
              ]
            }
          }
        ]
      }
    }
  }

Write query to file
  $ echo 'query { packages { items { name } } }' > query.gql

Run the query
  $ ${TURBO} query query.gql | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": {
        "items": [
          {
            "name": "//"
          },
          {
            "name": "another"
          },
          {
            "name": "my-app"
          },
          {
            "name": "util"
          }
        ]
      }
    }
  }