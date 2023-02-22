Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

# Build in stream order. All the .*'s are unpredictable lines, however the amount of lines is predictable.
  $ ${TURBO} run build --log-order stream --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  (my-app|util):build: cache bypass, force executing (f1ea8c68bf163f6b|8107080a88b155ef) (re)
  (my-app|util):build: cache bypass, force executing (f1ea8c68bf163f6b|8107080a88b155ef) (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  util:build: building
  my-app:build: done
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  