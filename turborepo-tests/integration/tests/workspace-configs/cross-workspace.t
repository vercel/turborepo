Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config
  $ ${TURBO} run cross-workspace-task --filter=cross-workspace
  \xe2\x80\xa2 Packages in scope: cross-workspace (esc)
  \xe2\x80\xa2 Running cross-workspace-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  blank-pkg:cross-workspace-underlying-task: cache miss, executing 13256295c40d6836
  blank-pkg:cross-workspace-underlying-task: 
  blank-pkg:cross-workspace-underlying-task: > cross-workspace-underlying-task
  blank-pkg:cross-workspace-underlying-task: > echo cross-workspace-underlying-task from blank-pkg
  blank-pkg:cross-workspace-underlying-task: 
  blank-pkg:cross-workspace-underlying-task: cross-workspace-underlying-task from blank-pkg
  cross-workspace:cross-workspace-task: cache miss, executing f386c2eceecb7035
  cross-workspace:cross-workspace-task: 
  cross-workspace:cross-workspace-task: > cross-workspace-task
  cross-workspace:cross-workspace-task: > echo cross-workspace-task
  cross-workspace:cross-workspace-task: 
  cross-workspace:cross-workspace-task: cross-workspace-task
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
