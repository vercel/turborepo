Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

TURBO_LOG_VERBOSITY should be respoected
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} daemon status
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::daemon::connector: looking for pid in lockfile: .+ (re)
  Turbo error: unable to connect: daemon is not running
  [1]

verbosity overrides TURBO_LOG_VERBOSITY global setting
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} daemon status -v
  Turbo error: unable to connect: daemon is not running
  [1]

verbosity doesn't override TURBO_LOG_VERBOSITY package settings
  $ TURBO_LOG_VERBOSITY=turborepo_lib=debug ${TURBO} daemon status -v
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::daemon::connector: looking for pid in lockfile: .+ (re)
  Turbo error: unable to connect: daemon is not running
  [1]
