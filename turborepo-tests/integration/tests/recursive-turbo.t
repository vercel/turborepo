Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

sed replaces the square brackets with parentheses so prysk can parse the file path
  $ ${TURBO} something 2>&1 | sed  's/\[\([^]]*\)\]/\(\1)/g'
  \xe2\x80\xa2 Packages in scope: //, another, my-app, util (esc)
  \xe2\x80\xa2 Running something in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
    x root task //#something (turbo run build) looks like it invokes turbo and
    | might cause a loop
     ,-\(.*package.json:3:1\) (re)
   3 |   "scripts": {
   4 |     "something": "turbo run build"
     :                  ^^^^^^^^|^^^^^^^^
     :                          `-- task found here
   5 |   },
     `----
  




