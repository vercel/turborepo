Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/no_workspaces_setup.sh $(pwd)/no_workspaces

  $ cd $TARGET_DIR && ${TURBO} run build --filter=nothing
    x No package found with name 'nothing' in workspace
  
  [1]

  $ cd $TARGET_DIR/parent && ${TURBO} run build --filter=nothing
    x No package found with name 'nothing' in workspace
  
  [1]
  $ cd $TARGET_DIR/parent/child && ${TURBO} run build --filter=nothing
    x No package found with name 'nothing' in workspace
  
  [1]
