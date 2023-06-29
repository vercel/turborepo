# Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) ordered

# Build as if we are in Github Actions
Note that we need to use (re) for lines that start with '> '
because otherwise prysk interprets them as multiline commands
  $ export GITHUB_ACTIONS=1
  $ ${TURBO} run build --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ::group::my-app:build
  cache bypass, force executing 4c3a4e8d472d74b2
  
  >\sbuild (re)
  >\secho 'building' && sleep 1 && echo 'done' (re)
  
  building
  done
  ::endgroup::
  ::group::util:build
  cache bypass, force executing 0bc36a8a234e31d4
  
  >\sbuild (re)
  >\ssleep 0.5 && echo 'building' && sleep 1 && echo 'completed' (re)
  
  building
  completed
  ::endgroup::
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
