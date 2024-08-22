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
  $ ${TURBO} query "query { packages(filter: { greaterThan: { field: DEPENDENT_COUNT, value: 0 } }) { name } }" | jq
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
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"util\" } }) { dependents { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "dependents": [
            {
              "name": "my-app"
            }
          ]
        }
      ]
    }
  }

Get dependencies of `my-app`
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { dependencies { name } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "packages": [
        {
          "dependencies": [
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