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
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "node_modules/repeat-string/index.js"
              },
              {
                "path": "button.tsx"
              },
              {
                "path": "foo.js"
              }
            ]
          }
        }
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"button.tsx\") { path, dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "button.tsx",
        "dependencies": {
          "files": {
            "items": []
          }
        }
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"circular.ts\") { path dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "circular.ts",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "circular2.ts"
              }
            ]
          }
        }
      }
    }
  }

Trace file with invalid import
  $ ${TURBO} query "query { file(path: \"invalid.ts\") { path dependencies { files { items { path } } errors { items { message } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "invalid.ts",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "button.tsx"
              }
            ]
          },
          "errors": {
            "items": [
              {
                "message": "failed to resolve import"
              }
            ]
          }
        }
      }
    }
  }

