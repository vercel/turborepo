
Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) task_dependencies/workspace-tasks

Test that root tasks are included in the graph. In this case, "//#build" task should be there
  $ ${TURBO} run build1 --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] //#build1" -> "[root] ___ROOT___" (esc)
  \t\t"[root] workspace-a#build1" -> "[root] ___ROOT___" (esc)
  \t\t"[root] workspace-b#build1" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  



Can depend on root tasks
  $ ${TURBO} run build2 --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] //#exists" -> "[root] ___ROOT___" (esc)
  \t\t"[root] ___ROOT___#build2" -> "[root] //#exists" (esc)
  \t\t"[root] workspace-a#build2" -> "[root] //#exists" (esc)
  \t\t"[root] workspace-a#build2" -> "[root] workspace-b#build2" (esc)
  \t\t"[root] workspace-b#build2" -> "[root] //#exists" (esc)
  \t\t"[root] workspace-b#build2" -> "[root] ___ROOT___#build2" (esc)
  \t} (esc)
  }
  


Can't depend on a missing root task
  $ ${TURBO} run build3 --graph
   ERROR  run failed: error preparing engine: //#not-exists needs an entry in turbo.json before it can be depended on because it is a task run from the root package
  Turbo error: error preparing engine: //#not-exists needs an entry in turbo.json before it can be depended on because it is a task run from the root package
  [1]

Package tasks can depend on things
  $ ${TURBO} run special --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] ___ROOT___#build4" -> "[root] ___ROOT___" (esc)
  \t\t"[root] workspace-a#special" -> "[root] workspace-b#build4" (esc)
  \t\t"[root] workspace-b#build4" -> "[root] ___ROOT___#build4" (esc)
  \t} (esc)
  }
  


