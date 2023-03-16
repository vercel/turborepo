Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "linked"

Make sure we use local and do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $TARGET_DIR "1.0.0"
  $ ${TURBO} build --filter foo -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: No local turbo binary found at: .*/linked/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: No local turbo binary found at: .*/linked/node_modules/turbo/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo path: .*/linked/node_modules/.pnpm/turbo-(darwin|linux|windows)-(64|arm64)@1.0.0/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo version: 1.0.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Repository Root: .*/linked (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Running local turbo binary in .*/linked/node_modules/.pnpm/turbo-(darwin|linux|windows)-(64|arm64)@1.0.0/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: supports_skip_infer_and_single_package false (re)
  build --filter foo --

Make sure we use local and pass --skip-infer to newer binary
  $ ${TESTDIR}/set_version.sh $TARGET_DIR "1.8.0"
  $ ${TURBO} build --filter foo -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: No local turbo binary found at: .*/linked/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: No local turbo binary found at: .*/linked/node_modules/turbo/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo path: .*/linked/node_modules/.pnpm/turbo-(darwin|linux|windows)-(64|arm64)@1.0.0/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::local_turbo_state: Local turbo version: 1.8.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Repository Root: .*/linked (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: Running local turbo binary in .*/linked/node_modules/.pnpm/turbo-(darwin|linux|windows)-(64|arm64)@1.0.0/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::state::turbo_state: supports_skip_infer_and_single_package true (re)
  --skip-infer build --filter foo --single-package --
