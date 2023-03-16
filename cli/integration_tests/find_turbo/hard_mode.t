Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/hard_mode_setup.sh $(pwd)/hard_mode/subdir "hoisted"
  $ TESTROOT=$(pwd)/hard_mode

When --skip-infer is used we use the current binary and output no global/local message
  $ cd $TESTROOT/subdir
  $ ${TURBO} --help --skip-infer -vv | head -n 1
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Global turbo version: .* (re)
  The build system that makes ship happen

It finds repo root and uses correct version
  $ ${TESTDIR}/set_version.sh $TESTROOT "1.8.0"
  $ cd $TESTROOT/subdir/node_modules
  $ ${TURBO} build --filter foo -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo path: .*/hard_mode/subdir/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo version: 1.8.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Repository Root: .*/hard_mode/subdir (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Running local turbo binary in .*/hard_mode/subdir/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: supports_skip_infer_and_single_package true (re)
  --skip-infer build --filter foo --single-package --

It respects cwd
  $ ${TESTDIR}/set_version.sh $TESTROOT "1.8.0"
  $ cd $TESTROOT
  $ ${TURBO} build --filter foo -vv --cwd ${TESTROOT}/subdir
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo path: .*/hard_mode/subdir/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo version: 1.8.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Repository Root: .*/hard_mode/subdir (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Running local turbo binary in .*/hard_mode/subdir/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: supports_skip_infer_and_single_package true (re)
  --skip-infer build --filter foo --single-package --

It respects cwd and finds repo root
  $ ${TESTDIR}/set_version.sh $TESTROOT "1.8.0"
  $ cd $TESTROOT
  $ ${TURBO} build --filter foo -vv --cwd ${TESTROOT}/subdir/node_modules
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo path: .*/hard_mode/subdir/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo version: 1.8.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Repository Root: .*/hard_mode/subdir (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Running local turbo binary in .*/hard_mode/subdir/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: supports_skip_infer_and_single_package true (re)
  --skip-infer build --filter foo --single-package --
