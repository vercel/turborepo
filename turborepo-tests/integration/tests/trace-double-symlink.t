Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh oxc_repro

  $ ${TURBO} query "query { file(path: \"./index.js\") { path dependencies { files { items { path } } errors { items { message import } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "index.js",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "nm(\/|\\\\)index.js" (re)
              }
            ]
          },
          "errors": {
            "items": []
          }
        }
      }
    }
  }
