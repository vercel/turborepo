Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

# Package inference does not find anything when run in the root.
  $ ${TURBO} build --skip-infer --force | grep "Running build in 3 packages" | head -n 1
  \xe2\x80\xa2 Running build in 3 packages (esc)

# Package inference succeeds when `--skip-infer` is passed and in a directory.
  $ cd packages/util
  $ ${TURBO} build --skip-infer --force | grep "Running build in 1 packages" | head -n 1
  \xe2\x80\xa2 Running build in 1 packages (esc)

# Does package inference when `--cwd` is passed.
  $ ${TURBO} build --cwd $(pwd)/packages/util --force | grep "Running build in 1 packages" | head -n 1
  \xe2\x80\xa2 Running build in 1 packages (esc)

# Does package inference when local `turbo`` detection is skipped and `--cwd` is passed.
  $ ${TURBO} build --skip-infer --cwd $(pwd)/packages/util --force | grep "Running build in 1 packages" | head -n 1
  \xe2\x80\xa2 Running build in 1 packages (esc)
