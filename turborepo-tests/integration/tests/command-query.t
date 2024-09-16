Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Query packages
  $ ${TURBO} query "query { packages { name } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
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

Query packages with equals filter
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { name } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "name": "my-app"
        }
      ]
    }
  }

Query packages that have at least one dependent package
  $ ${TURBO} query "query { packages(filter: { greaterThan: { field: DIRECT_DEPENDENT_COUNT, value: 0 } }) { name } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "name": "util"
        }
      ]
    }
  }

Get dependents of `util`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"util\" } }) { directDependents { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "directDependents": [
            {
              "name": "my-app"
            }
          ]
        }
      ]
    }
  }

Get dependencies of `my-app`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { directDependencies { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "directDependencies": [
            {
              "name": "util"
            }
          ]
        }
      ]
    }
  }

Get the indirect dependencies of `my-app`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { indirectDependencies { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "indirectDependencies": [
            {
              "name": "//"
            }
          ]
        }
      ]
    }
  }

Get all dependencies of `my-app`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { allDependencies { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "allDependencies": [
            {
              "name": "//"
            },
            {
              "name": "util"
            }
          ]
        }
      ]
    }
  }

Write query to file
  $ echo 'query { packages { name } }' > query.gql

Run the query
  $ ${TURBO} query query.gql | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
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