Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Query packages
  $ ${TURBO} query "query { packages { name } }"
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
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { name } }"
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
  $ ${TURBO} query "query { packages(filter: { greaterThan: { field: DEPENDENT_COUNT, value: 0 } }) { name } }"
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
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"util\" } }) { dependents { name } } }"
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
  $ ${TURBO} query "query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { dependencies { name } } }"
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