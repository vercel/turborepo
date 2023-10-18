Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package

Graph
  $ ${TURBO} run build --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
Graph in Rust
  $ ${TURBO} run build --graph --experimental-rust-codepath
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  