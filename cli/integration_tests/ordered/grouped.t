Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

# Build in grouped order.
  $ ${TURBO} run build --log-order grouped --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  (my-app|util):build: cache bypass, force executing (f1ea8c68bf163f6b|8107080a88b155ef) (re)
  (my-app|util):build: cache bypass, force executing (f1ea8c68bf163f6b|8107080a88b155ef) (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo 'building' && sleep 1 && echo 'done'
  my-app:build: 
  my-app:build: building
  my-app:build: done
  util:build: 
  util:build: > build
  util:build: > sleep 0.5 && echo 'building' && sleep 1 && echo 'completed'
  util:build: 
  util:build: building
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
