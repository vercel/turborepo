Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
  $ . ${TESTDIR}/../../helpers/replace_turbo_json.sh $(pwd) "interactive.json"
Verify we error on interactive task that hasn't been marked as cache: false
  $ ${TURBO} build
    x Tasks cannot be marked as interactive and cacheable
     ,-[turbo.json:7:1]
   7 |       "env": ["NODE_ENV"],
   8 |       "interactive": true,
     :                      ^^|^
     :                        `-- marked interactive here
   9 |       "outputs": []
     `----
  
  [1]
