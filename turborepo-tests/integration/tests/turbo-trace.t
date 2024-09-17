Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh create_turbo "npm@8.19.4"

  $ ${TURBO} query "query { file(path: \"apps/docs/app/page.tsx\") { path } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "apps/docs/app/page.tsx"
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"apps/docs/app/page.tsx\") { path, dependencies { path } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "apps/docs/app/page.tsx",
        "dependencies": [
          {
            "path": "apps/docs/app/page.tsx"
          },
          {
            "path": "node_modules/next/image.js"
          },
          {
            "path": "node_modules/react/index.js"
          },
          {
            "path": "packages/ui/src/button.tsx"
          }
        ]
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"apps/web/app/page.tsx\") { path, dependencies { path } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "apps/web/app/page.tsx",
        "dependencies": [
          {
            "path": "apps/web/app/page.tsx"
          },
          {
            "path": "node_modules/next/image.js"
          },
          {
            "path": "node_modules/react/index.js"
          },
          {
            "path": "packages/ui/src/button.tsx"
          }
        ]
      }
    }
  }
