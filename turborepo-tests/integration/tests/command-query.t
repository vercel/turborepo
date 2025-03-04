Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

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

  $ ${TURBO} query "query { version }" | jq ".data.version" > QUERY_VERSION
   WARNING  query command is experimental and may change in the future

  $ VERSION=${MONOREPO_ROOT_DIR}/version.txt
  $ diff --strip-trailing-cr <(head -n 1 ${VERSION}) <(${TURBO} --version)

