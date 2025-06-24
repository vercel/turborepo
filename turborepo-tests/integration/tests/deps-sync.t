Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Test deps-sync from subdirectory (should work)
  $ cd apps/my-app
  $ ${TURBO} deps-sync
  🔍 Scanning workspace packages for dependency conflicts...
  
  ✅ All dependencies are in sync!

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
        util (packages/util)
      4.17.21 →
        another (packages/another)
  
  
  \xe2\x9d\x8c Found 1 dependency conflicts. (no-eol) (esc)

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
        util (packages[\\/]util) (re)
      4.17.22 →
        my-app (apps[\\/]my-app) (re)
    typescript (version mismatch)
      5.0.0 →
        another (packages[\\/]another) (re)
        my-app (apps[\\/]my-app) (re)
      5.1.0 →
        util (packages[\\/]util) (re)
  
  
  \xe2\x9d\x8c Found 2 dependency conflicts. (no-eol) (esc)
  [1]

Test deps-sync with pinned dependencies
  $ . ${TESTDIR}/../../helpers/copy_fixture.sh $(pwd) deps_sync_pinned ${TESTDIR}/../fixtures

  $ ${TURBO} deps-sync
  🔍 Scanning workspace packages for dependency conflicts...
  
    lodash (pinned to 4.17.22)
      4.17.20 → util (packages[\\/]util) (re)
      4.17.21 → another (packages[\\/]another) (re)
  
  
  \xe2\x9d\x8c Found 1 dependency conflicts. (no-eol) (esc)
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