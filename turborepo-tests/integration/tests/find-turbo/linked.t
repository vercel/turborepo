Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "linked"

Make sure we use local, but do not pass --skip-infer to old binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.0.0"
  $ echo "Running turbo with verbose output..."
  Running turbo with verbose output...
  $ ${TURBO} build --filter foo -vv 2>&1 | tee out.log
  * (glob)
  $ echo "=== Full output from out.log ==="
  === Full output from out.log ===
  $ cat out.log
  * (glob)
  $ echo "=== Checking for version string ==="
  === Checking for version string ===
  $ grep -F "Local turbo version: 1.0.0" out.log || echo "VERSION STRING NOT FOUND"
  * (glob)
  $ echo "=== Last line of output ==="
  === Last line of output ===
  $ cat out.log | tail -n1
  build --filter foo -vv --

Make sure we use local, and DO pass --skip-infer to newer binary
  $ ${TESTDIR}/set_version.sh $(pwd) "1.8.0"
  $ echo "Running turbo with newer version..."
  Running turbo with newer version...
  $ ${TURBO} build --filter foo -vv 2>&1 | tee out.log
  * (glob)
  $ echo "=== Full output from out.log ==="
  === Full output from out.log ===
  $ cat out.log
  * (glob)
  $ echo "=== Checking for version string ==="
  === Checking for version string ===
  $ grep -F "Local turbo version: 1.8.0" out.log || echo "VERSION STRING NOT FOUND"
  * (glob)
  $ echo "=== Last line of output ==="
  === Last line of output ===
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --
