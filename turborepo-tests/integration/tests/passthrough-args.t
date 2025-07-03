Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh passthrough

  $ ${TURBO} -F my-app echo -- hello
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running echo in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:echo: cache miss, executing c0813f759149b8af
  my-app:echo: 
  my-app:echo: > echo
  my-app:echo: > echo hello
  my-app:echo: 
  my-app:echo: hello
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:    .*s  (re)
  

  $ ${TURBO} my-app#echo -- goodbye
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running my-app#echo in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:echo: cache miss, executing f4397252b3a3d780
  my-app:echo: 
  my-app:echo: > echo
  my-app:echo: > echo goodbye
  my-app:echo: 
  my-app:echo: goodbye
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:    .*s  (re)
  
