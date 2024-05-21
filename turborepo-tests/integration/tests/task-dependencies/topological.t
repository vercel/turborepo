Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/topological

Check my-app#build output
  $ ${TURBO} run build
  \xe2\x80\xa2 Packages in scope: //, my-app, util (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
<<<<<<< HEAD
  util:build: cache miss, executing c6b545f723eb2015
=======
  util:build: cache miss, executing 6eada9ec94009128
>>>>>>> 2eae5cbd82 (Update tests)
=======
  util:build: cache miss, executing b1a8d34ca4030bc7
>>>>>>> 37c3c596f1 (chore: update integration tests)
  util:build: 
  util:build: > build
  util:build: > echo building
  util:build: 
  util:build: building
<<<<<<< HEAD
<<<<<<< HEAD
  my-app:build: cache miss, executing b9d1448560566404
=======
  my-app:build: cache miss, executing 077c062fa59fb755
>>>>>>> 2eae5cbd82 (Update tests)
=======
  my-app:build: cache miss, executing 5e9c10768bcd20a8
>>>>>>> 37c3c596f1 (chore: update integration tests)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  




Graph
  $ ${TURBO} run build --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] //#build" -> "[root] ___ROOT___" (esc)
  \t\t"[root] my-app#build" -> "[root] util#build" (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  


