Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/topological

Check my-app#build output
  $ ${TURBO} run build
  \xe2\x80\xa2 Packages in scope: //, my-app, util (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing d8a403c3594c01e7
  util:build: 
  util:build: > build
  util:build: > echo building
  util:build: 
  util:build: building
  my-app:build: cache miss, executing 6f5a797f7b88130e
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
  


