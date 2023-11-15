Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)/subdir "hoisted"
  $ TESTROOT=$(pwd)

When --skip-infer is used we use the current binary and output no global/local message
  $ cd $TESTROOT/subdir
  $ ${TURBO} --help --skip-infer -vv | head -n 2
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  The build system that makes ship happen
  

It finds repo root and uses correct version
  $ ${TESTDIR}/set_version.sh $TESTROOT "1.8.0"
  $ cd $TESTROOT/subdir/node_modules
  $ ${TURBO} build --filter foo -vv > out.log 2>&1
  $ grep --quiet -F "Local turbo version: 1.8.0" out.log
  $ grep --quiet -E "Running local turbo binary in .*[\/\\]hard-mode.t[\/\\]subdir[\/\\]node_modules[\/\\].*[\/\\]bin[\/\\]turbo(\.exe)?" out.log
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --

It respects cwd
  $ ${TESTDIR}/set_version.sh $TESTROOT "1.8.0"
  $ cd $TESTROOT
  $ ${TURBO} build --filter foo -vv --cwd ${TESTROOT}/subdir > out.log 2>&1
  $ grep --quiet -F "Local turbo version: 1.8.0" out.log
  $ grep --quiet -E "Running local turbo binary in .*[\/\\]hard-mode.t[\/\\]subdir[\/\\]node_modules[\/\\].*[\/\\]bin[\/\\]turbo(\.exe)?" out.log
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --

It respects cwd and finds repo root
  $ ${TESTDIR}/set_version.sh $TESTROOT "1.8.0"
  $ cd $TESTROOT
  $ ${TURBO} build --filter foo -vv --cwd ${TESTROOT}/subdir/node_modules > out.log 2>&1
  $ grep --quiet -F "Local turbo version: 1.8.0" out.log
  $ grep --quiet -E "Running local turbo binary in .*[\/\\]hard-mode.t[\/\\]subdir[\/\\]node_modules[\/\\].*[\/\\]bin[\/\\]turbo(\.exe)?" out.log
  $ cat out.log | tail -n1
  --skip-infer build --filter foo -vv --single-package --
