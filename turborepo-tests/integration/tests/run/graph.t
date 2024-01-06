Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/topological

Graph to stdout
  $ ${TURBO} build -F my-app --graph
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] my-app#build" -> "[root] util#build" (esc)
  \t\t"[root] util#build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
  $ ${TURBO} build -F my-app --graph=graph.dot
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  
  .*Generated task graph in .*graph\.dot.* (re)
  $ cat graph.dot | grep -o "\"[^\"]*\" -> \"[^\"]*\""
  "[root] my-app#build" -> "[root] util#build"
  "[root] util#build" -> "[root] ___ROOT___"

  $ ${TURBO} build -F my-app --graph=graph.html
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  
  .*Generated task graph in .*graph\.html.* (re)
  $ cat graph.html | grep --quiet "DOCTYPE"

  $ ${TURBO} build -F my-app --graph=graph.mermaid
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  
  .*Generated task graph in .*graph\.mermaid.* (re)

  $ cat graph.mermaid
  graph TD
  \\t[A-Z]{4}\("my-app#build"\) --> [A-Z]{4}\("util#build"\).* (re)
  \\t[A-Z]{4}\("util#build"\) --> [A-Z]{4}\("___ROOT___"\).* (re)
