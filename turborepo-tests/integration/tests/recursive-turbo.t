Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

sed replaces the square brackets with parentheses so prysk can parse the file path
  $ ${TURBO} something 2>&1 | sed  's/\[\([^]]*\)\]/\(\1)/g'
  \xe2\x80\xa2 Packages in scope: //, another, my-app, util (esc)
  \xe2\x80\xa2 Running something in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  recursive_turbo_invocations (https://turborepo.com/messages/recursive-turbo-invocations)
  
    x Your `package.json` script looks like it invokes a Root Task (//
    | #something), creating a loop of `turbo` invocations. You likely have
    | misconfigured your scripts and tasks or your package manager's Workspace
    | structure.
     ,-\(.*package.json:4:18\) (re)
   3 |   "scripts": {
   4 |     "something": "turbo run build"
     :                  ^^^^^^^^|^^^^^^^^
     :                          `-- This script calls `turbo`, which calls the script, which calls `turbo`...
   5 |   },
     `----
  




