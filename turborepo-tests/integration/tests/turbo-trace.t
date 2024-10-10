Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh turbo_trace

  $ ${TURBO} query "query { file(path: \"main.ts\") { path } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "main.ts"
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"main.ts\") { path, dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "main.ts",
        "dependencies": [
          {
            "path": "button.tsx"
          },
          {
            "path": "foo.js"
          },
          {
            "path": "node_modules(\/|\\\\)repeat-string(\/|\\\\)index.js" (re)
          }
        ]
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"button.tsx\") { path, dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "button.tsx",
        "dependencies": []
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"circular.ts\") { path dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "circular.ts",
        "dependencies": [
          {
            "path": "circular2.ts"
          }
        ]
      }
    }
  }

Trace file with invalid import
  $ ${TURBO} query "query { file(path: \"invalid.ts\") { path dependencies { files { items { path } } errors { items { error } } } }"

