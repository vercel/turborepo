Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Graph
  $ ${TURBO} run test --graph
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] build" -> "[root] ___ROOT___" (esc)
  \t\t"[root] test" -> "[root] build" (esc)
  \t} (esc)
  }
  
