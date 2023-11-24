Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) npm

Populate cache
  $ ${TURBO} build --filter=a
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache miss, executing [0-9a-f]+ (re)
  a:build: 
  a:build: > build
  a:build: > echo building
  a:build: 
  a:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build --filter=b
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing [0-9a-f]+ (re)
  b:build: 
  b:build: > build
  b:build: > echo building
  b:build: 
  b:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Bump dependency for b and rebuild
Only b should have a cache miss
  $ patch package-lock.json package-lock.patch
  patching file package-lock.json
  $ ${TURBO} build  --filter=a
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache hit, replaying logs [0-9a-f]+ (re)
  a:build: 
  a:build: > build
  a:build: > echo building
  a:build: 
  a:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  

  $ ${TURBO} build  --filter=b
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing [0-9a-f]+ (re)
  b:build: 
  b:build: > build
  b:build: > echo building
  b:build: 
  b:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Add lockfile changes to a commit
  $ git add . && git commit -m "bump lockfile" --quiet
Only root and b should be rebuilt since only the deps for b had a version bump
  $ ${TURBO} build --filter="[HEAD^1]" --dry=json | jq ".packages"
  [
    "//",
    "b"
  ]
 
Bump of root workspace invalidates all packages
  $ patch package-lock.json turbo-bump.patch
  patching file package-lock.json
  $ ${TURBO} build  --filter=a
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache miss, executing [0-9a-f]+ (re)
  a:build: 
  a:build: > build
  a:build: > echo building
  a:build: 
  a:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build  --filter=b
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing [0-9a-f]+ (re)
  b:build: 
  b:build: > build
  b:build: > echo building
  b:build: 
  b:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Add lockfile changes to a commit
  $ git add . && git commit -m "global lockfile change" --quiet
Everything should be rebuilt as a dependency of the root package got bumped
  $ ${TURBO} build --filter="[HEAD^1]" --dry=json | jq ".packages | sort"
  [
    "//",
    "a",
    "b"
  ]
