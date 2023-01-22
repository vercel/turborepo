Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Output-logs full
  $ ${TURBO} run build --output-logs=full --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 7438505b97329a3d
  util:build: cache bypass, force executing 6dec18f9f767112f
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo 'building'
  my-app:build: 
  my-app:build: building
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:    333ms 
Output-logs none
  $ ${TURBO} run build --output-logs=none --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:    301ms 
   
Output-logs hash-only
  $ ${TURBO} run build --output-logs=hash-only --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 6dec18f9f767112f
  my-app:build: cache bypass, force executing 7438505b97329a3d
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:    304ms 

Output-logs new-only
  $ ${TURBO} run build --output-logs=new-only --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing 7438505b97329a3d
  util:build: cache bypass, force executing 6dec18f9f767112f
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo 'building'
  my-app:build: 
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  my-app:build: building
  util:build: building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:    301ms 
    
Output-logs error-only
  $ ${TURBO} run build --output-logs=error-only --force
  ERROR 'error-only' isn't a valid value for '--output-logs <OUTPUT_LOGS>'
    
  
    Did you mean 'errors-only'?
  
  For more information try '--help'
  
  [1]
Output-logs stdout-full
  $ ${TURBO} run build --output-logs=stdout-full --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 6dec18f9f767112f
  my-app:build: cache bypass, force executing 7438505b97329a3d
  
  [>] build
  [>] echo 'building'
  
  
  [>] build
  [>] echo 'building'
  
  building
  building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:    314ms 
  
Output-logs stdout-new-only
  $ ${TURBO} run build --output-logs=stdout-new-only --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 6dec18f9f767112f
  my-app:build: cache bypass, force executing 7438505b97329a3d
  
  [>] build
  [>] echo 'building'
  
  building
  
  [>] build
  [>] echo 'building'
  
  building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:    307ms 
