Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

# Running with --filter works and exits with success
  $ ${TURBO} run build --filter="[main]"
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  

# with unstaged changes
  $ echo "new file contents" >> bar.txt
  $ ${TURBO} run build --filter="[main]"
  \xe2\x80\xa2 Packages in scope: // (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  

  $ rm bar.txt
  $ echo "global dependency" >> foo.txt
  $ git commit -am "global dependency change" --quiet
  $ ${TURBO} run build --filter="[HEAD^]" --output-logs none
  \xe2\x80\xa2 Packages in scope: //, another, my-app, util (esc)
  \xe2\x80\xa2 Running build in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  

