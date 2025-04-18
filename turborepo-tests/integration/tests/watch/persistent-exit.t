Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh failing_dev

Turbo should exit after dev script fails
Disabling daemon so this matches behavior when running test on CI
  $ TURBO_DAEMON=0 ${TURBO} watch dev
   WARNING  daemon is required for watch, ignoring request to disable daemon
  \xe2\x80\xa2 Packages in scope: web (esc)
  \xe2\x80\xa2 Running dev in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  web:dev: cache bypass, force executing bfb830bdb7d49cb8
  web:dev: 
  web:dev: > dev
  web:dev: > echo server crashed && exit 1
  web:dev: 
  web:dev: server crashed
  web:dev: npm ERR! Lifecycle script `dev` failed with error: 
  web:dev: npm ERR! Error: command failed 
  web:dev: npm ERR!   in workspace: web 
  web:dev: npm ERR!   at location: .* (re)
  web:dev: ERROR: command finished with error: command .*npm(?:\.cmd)? run dev exited \(1\) (re)
  web#dev: command .*npm(?:\.cmd)? run dev exited \(1\) (re)
  web:dev: cache bypass, force executing bfb830bdb7d49cb8
  web:dev: 
  web:dev: > dev
  web:dev: > echo server crashed && exit 1
  web:dev: 
  web:dev: server crashed
  web:dev: npm ERR! Lifecycle script `dev` failed with error: 
  web:dev: npm ERR! Error: command failed 
  web:dev: npm ERR!   in workspace: web 
  web:dev: npm ERR!   at location: .* (re)
  web:dev: ERROR: command finished with error: command .*npm(?:\.cmd)? run dev exited \(1\) (re)
  web#dev: command .*npm(?:\.cmd)? run dev exited \(1\) (re)
    x persistent tasks exited unexpectedly
  
  [1]
