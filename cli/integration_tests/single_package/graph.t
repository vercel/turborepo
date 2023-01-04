Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Graph
  $ ${TURBO} run build --single-package --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
