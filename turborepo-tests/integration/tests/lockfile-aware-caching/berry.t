Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) berry

Populate cache
  $ ${TURBO} build --filter=a
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache miss, executing [0-9a-f]+ (re)
  a:build: yarn run v1.22.19
  a:build: warning package.json: No license field
  a:build: $ echo building
  a:build: building
  a:build: Done in 0.03s.
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build --filter=b
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing [0-9a-f]+ (re)
  b:build: yarn run v1.22.19
  b:build: warning package.json: No license field
  b:build: $ echo building
  b:build: building
  b:build: Done in 0.03s.
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Bump dependency for b and rebuild
Only b should have a cache miss
  $ patch yarn.lock yarn-lock.patch
  patching file yarn.lock
  $ ${TURBO} build  --filter=a
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache hit, replaying logs [0-9a-f]+ (re)
  a:build: yarn run v1.22.19
  a:build: warning package.json: No license field
  a:build: $ echo building
  a:build: building
  a:build: Done in 0.03s.
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  

  $ ${TURBO} build  --filter=b
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing [0-9a-f]+ (re)
  b:build: yarn run v1.22.19
  b:build: warning package.json: No license field
  b:build: $ echo building
  b:build: building
  b:build: Done in 0.03s.
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Add lockfile changes to a commit
  $ git add . && git commit -m "bump lockfile" --quiet
Only root and b should be rebuilt since only the deps for b had a version bump
  $ ${TURBO} build --filter="[HEAD^1]" --dry=json | jq ".packages"
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  [
    "//",
    "b"
  ]
 
Bump of root workspace invalidates all packages
  $ patch yarn.lock turbo-bump.patch
  patching file yarn.lock
  $ ${TURBO} build  --filter=a
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  \xe2\x80\xa2 Packages in scope: a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  a:build: cache miss, executing [0-9a-f]+ (re)
  a:build: yarn run v1.22.19
  a:build: warning package.json: No license field
  a:build: $ echo building
  a:build: building
  a:build: Done in 0.03s.
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build  --filter=b
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  \xe2\x80\xa2 Packages in scope: b (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  b:build: cache miss, executing [0-9a-f]+ (re)
  b:build: yarn run v1.22.19
  b:build: warning package.json: No license field
  b:build: $ echo building
  b:build: building
  b:build: Done in 0.03s.
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Add lockfile changes to a commit
  $ git add . && git commit -m "global lockfile change" --quiet
Everything should be rebuilt as a dependency of the root package got bumped
  $ ${TURBO} build --filter="[HEAD^1]" --dry=json | jq ".packages | sort"
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
  [
    "//",
    "a",
    "b"
  ]
