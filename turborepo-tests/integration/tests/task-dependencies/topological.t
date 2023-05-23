Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) task_dependencies/topological

Check my-app#build output
  $ ${TURBO} run build
  \xe2\x80\xa2 Packages in scope: //, my-app, util (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing 00b3a7311a76b73a
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  my-app:build: cache miss, executing 54b48304a67d52cf
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo 'building'
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
  \t\t"[root] ___ROOT___#build" -> "[root] ___ROOT___" (esc)
  \t\t"[root] my-app#build" -> "[root] util#build" (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___#build" (esc)
  \t} (esc)
  }
  


