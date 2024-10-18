Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh inference/has_workspaces

  $ cd $TARGET_DIR && ${TURBO} run build --filter=nothing -vv 1> ROOT 2>&1
  [1]
  $ grep --quiet 'pkg_inference_root set' ROOT
  [1]
  $ grep --quiet "No package found with name 'nothing' in workspace" ROOT

  $ cd $TARGET_DIR/apps/web && ${TURBO} run build --filter=nothing -vv 1> WEB 2>&1
  [1]
  $ grep --quiet 'pkg_inference_root set to "apps[\/\\]web"' WEB
  $ grep --quiet "No package found with name 'nothing' in workspace" WEB

  $ cd $TARGET_DIR/crates && ${TURBO} run build --filter=nothing -vv 1> CRATES 2>&1
  [1]
  $ grep --quiet 'pkg_inference_root set to "crates"' CRATES
  $ grep --quiet "No package found with name 'nothing' in workspace" CRATES

  $ cd $TARGET_DIR/crates/super-crate/tests/test-package && ${TURBO} run build --filter=nothing -vv 1> TEST_PACKAGE 2>&1
  [1]
  $ grep --quiet -E 'pkg_inference_root set to "crates[\/\\]super-crate[\/\\]tests[\/\\]test-package"' TEST_PACKAGE
  $ grep --quiet "No package found with name 'nothing' in workspace" TEST_PACKAGE

  $ cd $TARGET_DIR/packages/ui-library/src && ${TURBO} run build --filter=nothing -vv 1> UI_LIBRARY 2>&1
  [1]
  $ grep --quiet -E 'pkg_inference_root set to "packages[\/\\]ui-library[\/\\]src"' UI_LIBRARY
  $ grep --quiet "No package found with name 'nothing' in workspace" UI_LIBRARY
