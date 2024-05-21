This tests asserts that root tasks can depend on workspace#task
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/root-to-workspace

  $ ${TURBO} run mytask
  \xe2\x80\xa2 Packages in scope: //, lib-a (esc)
  \xe2\x80\xa2 Running mytask in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  lib-a:build: cache miss, executing cd724fed7c588f9d
=======
  lib-a:build: cache miss, executing db761418e4694c27
>>>>>>> 37c3c596f1 (chore: update integration tests)
  lib-a:build: 
  lib-a:build: > build
  lib-a:build: > echo build-lib-a
  lib-a:build: 
  lib-a:build: build-lib-a
<<<<<<< HEAD
  //:mytask: cache miss, executing 4998aef8c3bccdea
=======
  //:mytask: cache miss, executing 84287c0a1876a283
>>>>>>> 37c3c596f1 (chore: update integration tests)
  //:mytask: 
  //:mytask: > mytask
  //:mytask: > echo root-mytask
  //:mytask: 
  //:mytask: root-mytask
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9ms]+  (re)
  
