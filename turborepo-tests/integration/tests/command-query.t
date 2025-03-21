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

Query a file
  $ ${TURBO} query "query { file(path: \"apps/my-app/package.json\") { path, contents } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "apps(\/|\\\\)my-app(\/|\\\\)package.json", (re)
        "contents": "{\n  \"name\": \"my-app\",\n  \"scripts\": {\n    \"build\": \"echo building\",\n    \"maybefails\": \"exit 4\"\n  },\n  \"dependencies\": {\n    \"util\": \"*\"\n  }\n}\n"
      }
    }
  }

Get the file's package
  $ ${TURBO} query "query { file(path: \"apps/my-app/package.json\") { path, package { ... on Package { name } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "apps(\/|\\\\)my-app(\/|\\\\)package.json", (re)
        "package": {
          "name": "my-app"
        }
      }
    }
  }
