Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Test deps-sync on single package workspace (should error)
  $ cd apps/my-app
  $ ${TURBO} deps-sync
  Error: deps-sync is not needed for single-package workspaces. This command analyzes dependency conflicts across multiple packages in a workspace.
  [1]

Test deps-sync with no conflicts (basic monorepo)
  $ cd ../..
  $ ${TURBO} deps-sync
  🔍 Scanning workspace packages for dependency conflicts...
  
  ✅ All dependencies are in sync!

Test deps-sync with version conflicts
  $ . ${TESTDIR}/../../helpers/copy_fixture.sh $(pwd) deps_sync_version_conflicts ${TESTDIR}/../fixtures

  $ ${TURBO} deps-sync
  🔍 Scanning workspace packages for dependency conflicts...
  
    lodash (version mismatch)
      4.17.20 →
        util (packages[/\\]util) (re)
      4.17.21 →
        another (packages[/\\]another) (re)
  
  ❌ Found 1 dependency conflicts.
  [1]

Test deps-sync with allowlist generation
  $ ${TURBO} deps-sync --allowlist
  🔍 Scanning workspace packages for dependency conflicts...
  
  ✅ Generated allowlist configuration for 1 conflicts in turbo.json. Dependencies are now synchronized!

Verify allowlist was written to turbo.json
  $ cat turbo.json
  {
    "pipeline": {
      "build": {
        "outputs": [
          "dist/**"
        ]
      },
      "dev": {
        "cache": false
      }
    },
    "depsSync": {
      "ignoredDependencies": {
        "lodash": {
          "exceptions": [
            "another",
            "util"
          ]
        }
      }
    }
  }

Test deps-sync with allowlist in place (should pass)
  $ ${TURBO} deps-sync
  🔍 Scanning workspace packages for dependency conflicts...
  
  ✅ All dependencies are in sync!

Test deps-sync with mixed dependency types
  $ . ${TESTDIR}/../../helpers/copy_fixture.sh $(pwd) deps_sync_mixed_types ${TESTDIR}/../fixtures

  $ ${TURBO} deps-sync
  🔍 Scanning workspace packages for dependency conflicts...
  
    lodash (version mismatch)
      4.17.20 →
        util (packages[/\\]util) (re)
      4.17.22 →
        my-app (apps[/\\]my-app) (re)
    typescript (version mismatch)
      5.0.0 →
        another (packages[/\\]another) (re)
        my-app (apps[/\\]my-app) (re)
      5.1.0 →
        util (packages[/\\]util) (re)
  
  ❌ Found 2 dependency conflicts.
  [1]

Test deps-sync with pinned dependencies
  $ . ${TESTDIR}/../../helpers/copy_fixture.sh $(pwd) deps_sync_pinned ${TESTDIR}/../fixtures

  $ ${TURBO} deps-sync
  🔍 Scanning workspace packages for dependency conflicts...
  
    lodash (pinned to 4.17.22)
      4.17.20 → util (packages[/\\]util) (re)
      4.17.21 → another (packages[/\\]another) (re)
  
  ❌ Found 1 dependency conflicts.
  [1]

Test deps-sync with allowlist for pinned dependencies
  $ ${TURBO} deps-sync --allowlist
  🔍 Scanning workspace packages for dependency conflicts...
  
  ✅ Generated allowlist configuration for 1 conflicts in turbo.json. Dependencies are now synchronized!

Verify pinned dependency exceptions were added
  $ cat turbo.json
  {
    "pipeline": {
      "build": {
        "outputs": [
          "dist/**"
        ]
      },
      "dev": {
        "cache": false
      }
    },
    "depsSync": {
      "pinnedDependencies": {
        "lodash": {
          "version": "4.17.22",
          "exceptions": [
            "another",
            "util"
          ]
        }
      }
    }
  }

Test help text
  $ ${TURBO} deps-sync --help
  Analyze dependency conflicts across workspace packages
  
  Usage: turbo deps-sync [OPTIONS]
  
  Options:
        --allowlist  Generate an allowlist configuration to resolve conflicts
    -h, --help       Print help

Test invalid flag
  $ ${TURBO} deps-sync --invalid-flag
  error: unexpected argument '--invalid-flag' found
  
  Usage: turbo deps-sync [OPTIONS]
  
  For more information, try '--help'.
  [2]
