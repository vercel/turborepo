Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Verbosity level 1
  $ ${TURBO} build -v --filter=util --force
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 9b9969f14caa05a4
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build --verbosity=1 --filter=util --force
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 9b9969f14caa05a4
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Verbosity level 2
  $ ${TURBO} build -vv --filter=util --force
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::task_hash: task hash env vars for util:build (re)
   vars: \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::task_graph::visitor: task util#build hash is 9b9969f14caa05a4 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: rust (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["util"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:util fromRef: toRefOverride: raw:util}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  util   util}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:util fromRef: toRefOverride: raw:util}" entryPackages=map\[util:util] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[util:util] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[util:util] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=2107205f32600eb9 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hash env vars for util:build: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo.: start (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hash: value=9b9969f14caa05a4 (re)
  util:build: cache bypass, force executing 9b9969f14caa05a4
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  [-0-9:.TWZ+]+ \[DEBUG] turbo.: caching output: outputs="{\[packages/util/.turbo/turbo-build.log] \[]}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo.: done: status=complete duration=[\.0-9]+m?s (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build --verbosity=2 --filter=util --force
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::task_hash: task hash env vars for util:build (re)
   vars: []
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::task_graph::visitor: task util#build hash is 9b9969f14caa05a4 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: rust (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["util"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:util fromRef: toRefOverride: raw:util}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  util   util}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:util fromRef: toRefOverride: raw:util}" entryPackages=map\[util:util] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[util:util] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[util:util] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=2107205f32600eb9 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hash env vars for util:build: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo.: start (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hash: value=9b9969f14caa05a4 (re)
  util:build: cache bypass, force executing 9b9969f14caa05a4
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  [-0-9:.TWZ+]+ \[DEBUG] turbo.: caching output: outputs="{\[packages/util/.turbo/turbo-build.log] \[]}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo.: done: status=complete duration=[\.0-9]+m?s (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Make sure users can only use one verbosity flag
  $ ${TURBO} build -v --verbosity=1
   ERROR  the argument '-v...' cannot be used with '--verbosity <COUNT>'
  
  Usage: turbo [OPTIONS] [COMMAND]
  
  For more information, try '--help'.
  
  [1]

TURBO_LOG_VERBOSITY should be respoected
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} daemon status
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::daemon::connector: looking for pid in lockfile: .+ (re)
  Turbo error: unable to connect: daemon is not running
  [1]

verbosity overrides TURBO_LOG_VERBOSITY global setting
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} daemon status -v
  Turbo error: unable to connect: daemon is not running
  [1]

verbosity doesn't override TURBO_LOG_VERBOSITY package settings
  $ TURBO_LOG_VERBOSITY=turborepo_lib=debug ${TURBO} daemon status -v
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::daemon::connector: looking for pid in lockfile: .+ (re)
  Turbo error: unable to connect: daemon is not running
  [1]
