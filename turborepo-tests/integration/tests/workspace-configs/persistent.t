Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

This test covers:
- [x] `persistent:true` in root, omit in workspace with turbo.json
- [x] `persistent:true` in root, override to `false` in workspace
- [x] `persistent:true` in root, task exists in workspace, but doesn't touch persistent
- [x] No `persistent` flag in workspace, add `true` in workspace

# persistent-task-1-parent dependsOn persistent-task-1
# persistent-task-1 is persistent:true in the root workspace, and does NOT get overriden in the workspace
  $ ${TURBO} run persistent-task-1-parent --filter=persistent
    x invalid task configuration
  
  Error:   x "persistent#persistent-task-1" is a persistent task,
    | "persistent#persistent-task-1-parent" cannot depend on it
      ,-[turbo.json:88:1]
   88 |       "dependsOn": [
   89 |         "persistent-task-1"
      :         ^^^^^^^^^|^^^^^^^^^
      :                  `-- persistent task
   90 |       ]
      `----
  
  [1]

# persistent-task-2-parent dependsOn persistent-task-2
# persistent-task-2 is persistent:true in the root workspace, and IS overriden to false in the workspace
  $ ${TURBO} run persistent-task-2-parent --filter=persistent
  \xe2\x80\xa2 Packages in scope: persistent (esc)
  \xe2\x80\xa2 Running persistent-task-2-parent in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  persistent:persistent-task-2: cache miss, executing 7f08cefb45c613d2
  persistent:persistent-task-2: 
  persistent:persistent-task-2: > persistent-task-2
  persistent:persistent-task-2: > echo persistent-task-2
  persistent:persistent-task-2: 
  persistent:persistent-task-2: persistent-task-2
  persistent:persistent-task-2-parent: cache miss, executing affde90eaca06703
  persistent:persistent-task-2-parent: 
  persistent:persistent-task-2-parent: > persistent-task-2-parent
  persistent:persistent-task-2-parent: > echo persistent-task-2-parent
  persistent:persistent-task-2-parent: 
  persistent:persistent-task-2-parent: persistent-task-2-parent
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# persistent-task-3-parent dependsOn persistent-task-3
# persistent-task-3 is persistent:true in the root workspace
# persistent-task-3 is defined in workspace, but does NOT have the persistent flag
  $ ${TURBO} run persistent-task-3-parent --filter=persistent
    x invalid task configuration
  
  Error:   x "persistent#persistent-task-3" is a persistent task,
    | "persistent#persistent-task-3-parent" cannot depend on it
       ,-[turbo.json:98:1]
    98 |       "dependsOn": [
    99 |         "persistent-task-3"
       :         ^^^^^^^^^|^^^^^^^^^
       :                  `-- persistent task
   100 |       ]
       `----
  
  [1]

# persistent-task-4-parent dependsOn persistent-task-4
# persistent-task-4 has no config in the root workspace, and is set to true in the workspace
  $ ${TURBO} run persistent-task-4-parent --filter=persistent
    x invalid task configuration
  
  Error:   x "persistent#persistent-task-4" is a persistent task,
    | "persistent#persistent-task-4-parent" cannot depend on it
       ,-[turbo.json:103:1]
   103 |       "dependsOn": [
   104 |         "persistent-task-4"
       :         ^^^^^^^^^|^^^^^^^^^
       :                  `-- persistent task
   105 |       ]
       `----
  
  [1]
