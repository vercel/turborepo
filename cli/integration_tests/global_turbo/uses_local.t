Setup
  $ . ${TESTDIR}/../setup.sh

Make sure we use local and don't pass --skip-infer to old binary
  $ . ${TESTDIR}/setup.sh 1.2.3
  $ ${TURBO} build --filter foo
  Running local turbo binary in .*node_modules/.bin/turbo (re)
  
  build --filter foo --

Make sure we use local and pass --skip-infer to newer binary
  $ . ${TESTDIR}/setup.sh 1.8.9
  $ ${TURBO} build --filter foo
  Running local turbo binary in .*node_modules/.bin/turbo (re)
  
  --skip-infer build --filter foo --single-package --

It finds repo root and uses correct version
  $ cd subdir
  $ ${TURBO} build --filter foo
  Running local turbo binary in .*node_modules/.bin/turbo (re)
  
  --skip-infer build --filter foo --single-package --
  $ cd ..

It respects cwd
  $ ROOT=$(pwd); cd ..
  $ ${TURBO} build --filter foo --cwd ${ROOT}
  Running local turbo binary in .*node_modules/.bin/turbo (re)
  
  --skip-infer build --filter foo --single-package --

It respects cwd and finds repo root
  $ ${TURBO} build --filter foo --cwd ${ROOT}/subdir
  Running local turbo binary in .*node_modules/.bin/turbo (re)
  
  --skip-infer build --filter foo --single-package --
