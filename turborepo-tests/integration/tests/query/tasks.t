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

  $ ${TURBO} query "query { package(name: \"my-app\") { tasks { items { name directDependencies { items { name } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "name": "build",
              "directDependencies": {
                "items": []
              }
            }
          ]
        }
      }
    }
  }