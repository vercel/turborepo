Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Run trap and immediately sent INT to it
  $ ${TURBO} trap &
  $ TURBO_PID=$!
  $ sh -c "sleep 1 && kill -2 ${TURBO_PID}" &
  \xe2\x80\xa2 Packages in scope: test (esc)
  \xe2\x80\xa2 Running trap in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  test:trap: cache miss, executing d25759ee0a8e12ae
  test:trap: 
  test:trap: > trap
  test:trap: > trap 'echo trap hit; sleep 1; echo trap finish' INT; sleep 5 && echo 'script finish'
  test:trap: 
  test:trap: trap hit
  test:trap: trap finish
  test:trap: npm ERR! Lifecycle script `trap` failed with error: 
  test:trap: npm ERR! Error: command failed 
  test:trap: npm ERR!   in workspace: test 
  test:trap: npm ERR!   at location: .*ctrlc.t/apps/test  (re)  (no-eol)
