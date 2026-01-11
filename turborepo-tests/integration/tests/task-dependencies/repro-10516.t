Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/repro-10516

Test reproduction of issue 10516
  $ ${TURBO} run dev --graph
  
  digraph {
	compound = "true"
	newrank = "true"
	subgraph "root" {
		" workspace-a#build:openapi" -> " ___ROOT___"
		" workspace-a#dev" -> " workspace-a#build:openapi"
		" workspace-b#dev" -> " workspace-a#build:openapi"
	}
  }
