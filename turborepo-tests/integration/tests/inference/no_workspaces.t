Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/no_workspaces_setup.sh $(pwd)/no_workspaces

  $ cd $TARGET_DIR && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/no_workspaces (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: no hash \(single package\) (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: rust (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["nothing"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  nothing   nothing}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" entryPackages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=070330d74e4b29de (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ cd $TARGET_DIR/parent && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/no_workspaces/parent (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: no hash \(single package\) (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: (go|rust) (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["nothing"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  nothing   nothing}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" entryPackages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=908de86919a5f21a (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ cd $TARGET_DIR/parent/child && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/no_workspaces/parent/child (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: no hash \(single package\) (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: (go|rust) (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["nothing"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  nothing   nothing}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" entryPackages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=a482b5cbf06694fd (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  