Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) task_dependencies/topological

Graph to stdout
  $ ${TURBO} build -F my-app --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] //#build" -> "[root] ___ROOT___" (esc)
  \t\t"[root] my-app#build" -> "[root] util#build" (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
  $ ${TURBO} build -F my-app --graph=graph.dot
  
  .*Generated task graph in .*graph\.dot.* (re)
  $ cat graph.dot
  digraph {
  \tgraph [bb="0,0,300.79,180", (esc)
  \t\tcompound=true, (esc)
  \t\tnewrank=true (esc)
  \t]; (esc)
  \tnode [label="\\N"]; (esc)
  \tsubgraph root { (esc)
  \t\t"[root] //#build"\t[height=0.5, (esc)
  \t\t\tpos="63.123,90", (esc)
  \t\t\twidth=1.7534]; (esc)
  \t\t"[root] ___ROOT___"\t[height=0.5, (esc)
  \t\t\tpos="138.12,18", (esc)
  \t\t\twidth=2.4216]; (esc)
  \t\t"[root] //#build" -> "[root] ___ROOT___"\t[pos="e,119.96,35.956 80.897,72.411 90.194,63.734 101.75,52.946 112.04,43.342"]; (esc)
  \t\t"[root] my-app#build"\t[height=0.5, (esc)
  \t\t\tpos="214.12,162", (esc)
  \t\t\twidth=2.4074]; (esc)
  \t\t"[root] util#build"\t[height=0.5, (esc)
  \t\t\tpos="214.12,90", (esc)
  \t\t\twidth=1.9525]; (esc)
  \t\t"[root] my-app#build" -> "[root] util#build"\t[pos="e,214.12,108.1 214.12,143.7 214.12,136.24 214.12,127.32 214.12,118.97"]; (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___"\t[pos="e,156.53,35.956 196.11,72.411 186.69,63.734 174.98,52.946 164.55,43.342"]; (esc)
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
