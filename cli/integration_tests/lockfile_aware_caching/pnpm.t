Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) pnpm

Populate cache
  $ ${TURBO} build --filter=a
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache miss, executing c50b1361a2e2ebf5
  a:build: 
  a:build: > a@ build .*/pnpm.t/apps/a (re)
  a:build: > echo 'building'
  a:build: 
  a:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build --filter=b
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing f8f391c807e6fed9
  b:build: 
  b:build: > b@ build .*/pnpm.t/apps/b (re)
  b:build: > echo 'building'
  b:build: 
  b:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Bump dependency for b and rebuild
Only b should have a cache miss
  $ patch pnpm-lock.yaml pnpm-lock.patch
  patching file pnpm-lock.yaml
  $ ${TURBO} build  --filter=a
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache hit, replaying output c50b1361a2e2ebf5
  a:build: 
  a:build: > a@ build .*/pnpm.t/apps/a (re)
  a:build: > echo 'building'
  a:build: 
  a:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  

  $ ${TURBO} build  --filter=b
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing e0a8a35a54508e7f
  b:build: 
  b:build: > b@ build .*/pnpm.t/apps/b (re)
  b:build: > echo 'building'
  b:build: 
  b:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
 
Bump of root workspace invalidates all packages
  $ patch pnpm-lock.yaml turbo-bump.patch
  patching file pnpm-lock.yaml
  $ ${TURBO} build  --filter=a
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache miss, executing 3d918a002d8810b4
  a:build: 
  a:build: > a@ build .*/pnpm.t/apps/a (re)
  a:build: > echo 'building'
  a:build: 
  a:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build  --filter=b
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing 1c240ac10038c94c
  b:build: 
  b:build: > b@ build .*/pnpm.t/apps/b (re)
  b:build: > echo 'building'
  b:build: 
  b:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
