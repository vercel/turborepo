Setup
  $ . ${TESTDIR}/../../../../helpers/setup.sh
  $ . ${TESTDIR}/../../_helpers/setup_monorepo.sh $(pwd) single_package

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
  $ cat graph.dot
  digraph {
  \tgraph [bb="0,0,174.36,108", (esc)
  \t\tcompound=true, (esc)
  \t\tnewrank=true (esc)
  \t]; (esc)
  \tnode [label="\\N"]; (esc)
  \tsubgraph root { (esc)
  \t\t"[root] build"\t[height=0.5, (esc)
  \t\t\tpos="87.178,90", (esc)
  \t\t\twidth=1.4833]; (esc)
  \t\t"[root] ___ROOT___"\t[height=0.5, (esc)
  \t\t\tpos="87.178,18", (esc)
  \t\t\twidth=2.4216]; (esc)
  \t\t"[root] build" -> "[root] ___ROOT___"\t[pos="e,87.178,36.104 87.178,71.697 87.178,64.237 87.178,55.322 87.178,46.965"]; (esc)
  \t} (esc)
  }
