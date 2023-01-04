Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Graph
  $ ${TURBO} run build --single-package --graph
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
