Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "unplugged"

Make sure we use local and do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.0.0"
  $ ${TURBO} build --filter foo -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .*/unplugged.t/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .*/unplugged.t/node_modules/turbo/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Local turbo path: .*/unplugged.t/.yarn/unplugged/turbo-(darwin|linux|windows)-(64|arm64)-npm-1.0.0-520925a700/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Local turbo version: 1.0.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/unplugged.t (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running local turbo binary in .*/unplugged.t/.yarn/unplugged/turbo-(darwin|linux|windows)-(64|arm64)-npm-1.0.0-520925a700/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: supports_skip_infer_and_single_package false (re)
  build --filter foo -vv --

Make sure we use local and pass --skip-infer to newer binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.8.0"
  $ ${TURBO} build --filter foo -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .*/unplugged.t/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .*/unplugged.t/node_modules/turbo/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Local turbo path: .*/unplugged.t/.yarn/unplugged/turbo-(darwin|linux|windows)-(64|arm64)-npm-1.0.0-520925a700/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Local turbo version: 1.8.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/unplugged.t (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running local turbo binary in .*/unplugged.t/.yarn/unplugged/turbo-(darwin|linux|windows)-(64|arm64)-npm-1.0.0-520925a700/node_modules/turbo-(darwin|linux|windows)-(64|arm64)/bin/turbo (re)
  
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: supports_skip_infer_and_single_package true (re)
  --skip-infer build --filter foo -vv --single-package --
