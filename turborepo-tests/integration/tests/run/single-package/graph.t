Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Graph
  $ ${TURBO} run build --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
Graph file
  $ ${TURBO} build --graph=graph.dot
  
  .*Generated task graph in .*graph\.dot.* (re)
  $ cat graph.dot | grep -o "\"[^\"]*\" -> \"[^\"]*\""
  "[root] build" -> "[root] ___ROOT___"
