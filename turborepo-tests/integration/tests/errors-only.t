Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd) run_logging

# [ ] error exit
# [ ] outputMode: errors-only
# [x] --ouptut-logs=errors-only
  $ ${TURBO} run build --output-logs=errors-only
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  



# [ ] error exit
# [x] outputMode: errors-only
# [ ] --ouptut-logs=errors-only
  $ ${TURBO} run buildsuccess
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running buildsuccess in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  


# [x] error exit
# [ ] outputMode: errors-only
# [x] --ouptut-logs=errors-only
  $ ${TURBO} run builderror --output-logs=errors-only
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running builderror in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:builderror: cache miss, executing e66674cd52f90ec9
  app-a:builderror: 
  app-a:builderror: > builderror
  app-a:builderror: > echo "error builderror app-a!" && exit 1
  app-a:builderror: 
  app-a:builderror: error builderror app-a!
  app-a:builderror: npm ERR! Lifecycle script `builderror` failed with error: 
  app-a:builderror: npm ERR! Error: command failed 
  app-a:builderror: npm ERR!   in workspace: app-a 
  app-a:builderror: npm ERR!   at location: .* (re)
  app-a:builderror: ERROR: command finished with error: command .* npm run builderror exited \(1\) (re)
  command .* npm run builderror exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]



# [x] error exit
# [x] outputMode: errors-only
# [ ] --ouptut-logs=errors-only
  $ ${TURBO} run builderror2
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running builderror2 in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:builderror2: cache miss, executing de9a4f19fc30bada
  app-a:builderror2: 
  app-a:builderror2: > builderror2
  app-a:builderror2: > echo "error builderror2 app-a!" && exit 1
  app-a:builderror2: 
  app-a:builderror2: error builderror2 app-a!
  app-a:builderror2: npm ERR! Lifecycle script `builderror2` failed with error: 
  app-a:builderror2: npm ERR! Error: command failed 
  app-a:builderror2: npm ERR!   in workspace: app-a 
  app-a:builderror2: npm ERR!   at location: .* (re)
  app-a:builderror2: ERROR: command finished with error: command .* npm run builderror2 exited \(1\) (re)
  command .* npm run builderror2 exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]



