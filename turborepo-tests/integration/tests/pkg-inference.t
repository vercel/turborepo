Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

# Run as if called by global turbo (NEW)
  $ cd packages/util
  $ ${TURBO} build --skip-infer --force | grep "Running build in 1 packages" | head -n 1
  \xe2\x80\xa2 Running build in 1 packages (esc)

# Does not run in a particular directory
  $ ${TURBO} build --cwd $(pwd)/packages/util --force | grep "Running build in 1 packages" | head -n 1
  \xe2\x80\xa2 Running build in 1 packages (esc)

# Run in a particular directory
  $ ${TURBO} build --skip-infer --cwd $(pwd)/packages/util --force | grep "Running build in 1 packages" | head -n 1
  \xe2\x80\xa2 Running build in 1 packages (esc)
