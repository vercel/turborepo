Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) inference/has_workspaces

  $ cd $TARGET_DIR && ${TURBO} run build --filter=nothing -vv 1> ROOT 2>&1
  $ cat ROOT | grep --only-match 'pkg_inference_root set'
  [1]
  $ cat ROOT | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/apps/web && ${TURBO} run build --filter=nothing -vv 1> WEB 2>&1
  $ cat WEB | grep --only-match 'pkg_inference_root set to "apps/web"'
  pkg_inference_root set to "apps/web"
  $ cat WEB | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/crates && ${TURBO} run build --filter=nothing -vv 1> CRATES 2>&1
  $ cat CRATES | grep --only-match 'pkg_inference_root set to "crates"'
  pkg_inference_root set to "crates"
  $ cat CRATES | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/crates/super-crate/tests/test-package && ${TURBO} run build --filter=nothing -vv 1> TEST_PACKAGE 2>&1
  $ cat TEST_PACKAGE | grep --only-match 'pkg_inference_root set to "crates/super-crate/tests/test-package"'
  pkg_inference_root set to "crates/super-crate/tests/test-package"
  $ cat TEST_PACKAGE | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/packages/ui-library/src && ${TURBO} run build --filter=nothing -vv 1> UI_LIBRARY 2>&1
  $ cat UI_LIBRARY | grep --only-match 'pkg_inference_root set to "packages/ui-library/src"'
  pkg_inference_root set to "packages/ui-library/src"
  $ cat UI_LIBRARY | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.
