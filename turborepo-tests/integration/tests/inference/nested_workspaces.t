Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/nested_workspaces_setup.sh $(pwd)/nested_workspaces

  $ cd $TARGET_DIR/outer && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
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
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=00437efdb7e230f5 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ cd $TARGET_DIR/outer/apps && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "apps" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using apps as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: rust (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Using apps as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["nothing"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  nothing   nothing}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" entryPackages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=00437efdb7e230f5 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ cd $TARGET_DIR/outer/inner && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer/inner (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
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
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=00437efdb7e230f5 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ cd $TARGET_DIR/outer/inner/apps && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer/inner (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "apps" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using apps as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: rust (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Using apps as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["nothing"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  nothing   nothing}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" entryPackages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=00437efdb7e230f5 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
Locate a repository with no turbo.json. We'll get the right root, but there's nothing to run
  $ cd $TARGET_DIR/outer/inner-no-turbo && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer/inner-no-turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  Error: Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one
  [1]

Locate a repository with no turbo.json. We'll get the right root and inference directory, but there's nothing to run
  $ cd $TARGET_DIR/outer/inner-no-turbo/apps && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer/inner-no-turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "apps" (re)
  Error: Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one
  [1]

  $ cd $TARGET_DIR/outer-no-turbo && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer-no-turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  Error: Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one
  [1]

  $ cd $TARGET_DIR/outer-no-turbo/apps && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: [\-\w\/\.]+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: [\-\w\/\.]+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: [\-\w\/\.]+ (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "apps" (re)
  Error: Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one
  [1]

  $ cd $TARGET_DIR/outer-no-turbo/inner && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer-no-turbo/inner (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using  as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
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
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=00437efdb7e230f5 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ cd $TARGET_DIR/outer-no-turbo/inner/apps && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer-no-turbo/inner (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "apps" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: global hash env vars \[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::global_hash: external deps hash: 459c029558afe716 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::run::scope::filter: Using apps as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Found go binary at "[\-\w\/\.]+" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: build tag: rust (re)
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Using apps as a basis for selecting packages (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filter patterns: patterns=\["nothing"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Parsed selector: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtering packages: allPackageSelectors=\["&{false false false false false false  nothing   nothing}"] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: selector="&{includeDependencies:false matchDependencies:false includeDependents:false exclude:false excludeSelf:false followProdDepsOnly:false parentDir: namePattern:nothing fromRef: toRefOverride: raw:nothing}" entryPackages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: Filtered packages: cherryPickedPackages=map\[] walkedDependencies=map\[] walkedDependents=map\[] walkedDependentsDependencies=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: filtered packages: packages=map\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash env vars: vars=\[] (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash: value=00437efdb7e230f5 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: global hash matches between Rust and Go (re)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: local cache folder: path="" (re)
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  [-0-9:.TWZ+]+ \[DEBUG] turbo: task hashes match (re)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ cd $TARGET_DIR/outer-no-turbo/inner-no-turbo && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer-no-turbo/inner-no-turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  Error: Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one
  [1]

  $ cd $TARGET_DIR/outer-no-turbo/inner-no-turbo/apps && ${TURBO} run build --filter=nothing -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/nested_workspaces/outer-no-turbo/inner-no-turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "apps" (re)
  Error: Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one
  [1]
