
Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/workspace-tasks

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
  \t\t"[root] workspace-a#build2" -> "[root] //#exists" (esc)
  \t\t"[root] workspace-a#build2" -> "[root] workspace-b#build2" (esc)
  \t\t"[root] workspace-b#build2" -> "[root] //#exists" (esc)
  \t} (esc)
  }
  


Can't depend on a missing root task
  $ ${TURBO} run build3 --graph > BUILD3 2>&1
  [1]
  $ cat BUILD3 | grep --quiet --only-match 'x //#not-exists needs an entry in turbo.json before it can be depended on'
  $ cat BUILD3 | grep --quiet --only-match 'because it is a task declared in the root package.json'

Package tasks can depend on things
  $ ${TURBO} run special --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] workspace-a#special" -> "[root] workspace-b#build4" (esc)
  \t\t"[root] workspace-b#build4" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  


