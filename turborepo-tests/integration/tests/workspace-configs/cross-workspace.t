Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config
  $ ${TURBO} run cross-workspace-task --filter=cross-workspace
  \xe2\x80\xa2 Packages in scope: cross-workspace (esc)
  \xe2\x80\xa2 Running cross-workspace-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  blank-pkg:cross-workspace-underlying-task: cache miss, executing 6002174173495dbf
=======
  blank-pkg:cross-workspace-underlying-task: cache miss, executing 209948d9d34ba289
>>>>>>> 37c3c596f1 (chore: update integration tests)
  blank-pkg:cross-workspace-underlying-task: 
  blank-pkg:cross-workspace-underlying-task: > cross-workspace-underlying-task
  blank-pkg:cross-workspace-underlying-task: > echo cross-workspace-underlying-task from blank-pkg
  blank-pkg:cross-workspace-underlying-task: 
  blank-pkg:cross-workspace-underlying-task: cross-workspace-underlying-task from blank-pkg
<<<<<<< HEAD
  cross-workspace:cross-workspace-task: cache miss, executing 6dd8e4d2ceda14c4
=======
  cross-workspace:cross-workspace-task: cache miss, executing 3b29356496e9ae93
>>>>>>> 37c3c596f1 (chore: update integration tests)
  cross-workspace:cross-workspace-task: 
  cross-workspace:cross-workspace-task: > cross-workspace-task
  cross-workspace:cross-workspace-task: > echo cross-workspace-task
  cross-workspace:cross-workspace-task: 
  cross-workspace:cross-workspace-task: cross-workspace-task
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
