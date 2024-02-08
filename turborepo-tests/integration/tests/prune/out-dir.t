Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_with_root_dep pnpm@7.25.1

Test that absolute paths can be passed as out-dir
  $ TMPFILE=$(mktemp -d)
  $ ${TURBO} prune web --out-dir=${TMPFILE}
  Generating pruned monorepo for web in .* (re)
   - Added shared
   - Added util
   - Added web
  $ cat ${TMPFILE}/package.json
  {
    "name": "monorepo",
    "packageManager": "pnpm@7.25.1",
    "devDependencies": {
      "util": "workspace:*"
    },
    "pnpm": {
      "patchedDependencies": {
        "is-number@7.0.0": "patches/is-number@7.0.0.patch"
      }
    }
  }
