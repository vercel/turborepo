Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) task_dependencies/topological

Graph to stdout
  $ ${TURBO} build -F my-app --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] my-app#build" -> "[root] util#build" (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
  $ ${TURBO} build -F my-app --graph=graph.dot
  
  .*Generated task graph in .*graph\.dot.* (re)
  $ cat graph.dot
  digraph {
  \tgraph [bb="0,0,174.36,180", (esc)
  \t\tcompound=true, (esc)
  \t\tnewrank=true (esc)
  \t]; (esc)
  \tnode [label="\\N"]; (esc)
  \tsubgraph root { (esc)
  \t\t"[root] my-app#build"\t[height=0.5, (esc)
  \t\t\tpos="87.178,162", (esc)
  \t\t\twidth=2.4074]; (esc)
  \t\t"[root] util#build"\t[height=0.5, (esc)
  \t\t\tpos="87.178,90", (esc)
  \t\t\twidth=1.9525]; (esc)
  \t\t"[root] my-app#build" -> "[root] util#build"\t[pos="e,87.178,108.1 87.178,143.7 87.178,136.24 87.178,127.32 87.178,118.97"]; (esc)
  \t\t"[root] ___ROOT___"\t[height=0.5, (esc)
  \t\t\tpos="87.178,18", (esc)
  \t\t\twidth=2.4216]; (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___"\t[pos="e,87.178,36.104 87.178,71.697 87.178,64.237 87.178,55.322 87.178,46.965"]; (esc)
  \t} (esc)
  }

  $ ${TURBO} build -F my-app --graph=graph.html
  
  .*Generated task graph in .*graph\.html.* (re)
  $ cat graph.html | grep --quiet "DOCTYPE"

  $ ${TURBO} build -F my-app --graph=graph.mermaid
  
  .*Generated task graph in .*graph\.mermaid.* (re)

  $ cat graph.mermaid
  graph TD
  \\t[A-Z]{4}\("my-app#build"\) --> [A-Z]{4}\("util#build"\).* (re)
  \\t[A-Z]{4}\("util#build"\) --> [A-Z]{4}\("___ROOT___"\).* (re)
