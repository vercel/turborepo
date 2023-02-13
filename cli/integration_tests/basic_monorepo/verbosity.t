Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Verbosity level 1
  $ ${TURBO} build -v --filter=util --force
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 6dec18f9f767112f
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
  util:build: cache bypass, force executing 6dec18f9f767112f
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
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  2023-02-13T20:19:15.000+0000 [DEBUG] turborepo_lib: Args passed to Go binary:
  {
    "version": false,
    "api": null,
    "color": false,
    "cpu_profile": null,
    "cwd": "/private/var/folders/vg/sr4krlws0k12g21phhjwy4z40000gn/T/prysk-tests-1oo0sdqw/verbosity.t",
    "heap": null,
    "login": null,
    "no_color": false,
    "preflight": false,
    "team": null,
    "token": null,
    "trace": null,
    "verbosity": 2,
    "check_for_update": false,
    "test_run": false,
    "run_args": null,
    "command": {
      "Run": {
        "cache_dir": null,
        "cache_workers": 10,
        "concurrency": null,
        "continue_execution": false,
        "dry_run": null,
        "single_package": false,
        "filter": [
          "util"
        ],
        "force": true,
        "global_deps": [],
        "graph": null,
        "ignore": [],
        "include_dependencies": false,
        "no_cache": false,
        "no_daemon": false,
        "no_deps": false,
        "output_logs": null,
        "only": false,
        "parallel": false,
        "pkg_inference_root": "",
        "profile": null,
        "remote_only": false,
        "scope": [],
        "since": null,
        "tasks": [
          "build"
        ],
        "pass_through_args": []
      }
    }
  }
  2023-02-13T20:19:15.000+0000 [DEBUG] turbo: Found go binary at "/Users/mehulkar/dev/vercel/turbo/target/debug/go-turbo"
  2023-02-13T20:19:15.007Z [INFO]  turbo: skipping turbod since we appear to be in a non-interactive context
  2023-02-13T20:19:15.007Z [DEBUG] turbo: global hash env vars: vars=["VERCEL_ANALYTICS_ID"]
  2023-02-13T20:19:15.007Z [DEBUG] turbo: global hash: value=430b3790556340cb
  2023-02-13T20:19:15.007Z [DEBUG] turbo: local cache folder: path=""
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  2023-02-13T20:19:15.025Z [DEBUG] turbo.: start
  2023-02-13T20:19:15.025Z [DEBUG] turbo: task hash env vars for util:build: vars=[]
  2023-02-13T20:19:15.025Z [DEBUG] turbo: task hash: value=6dec18f9f767112f
  util:build: cache bypass, force executing 6dec18f9f767112f
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  2023-02-13T20:19:15.224Z [DEBUG] turbo.: caching output: outputs="{[packages/util/.turbo/turbo-build.log] []}"
  2023-02-13T20:19:15.224Z [DEBUG] turbo.: done: status=complete duration=199.366375ms
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:    217ms 
  
  $ ${TURBO} build --verbosity=2 --filter=util --force
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Running command as global turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::cli: pkg_inference_root set to "" (re)
  2023-02-13T20:19:15.239+0000 [DEBUG] turborepo_lib: Args passed to Go binary:
  {
    "version": false,
    "api": null,
    "color": false,
    "cpu_profile": null,
    "cwd": "/private/var/folders/vg/sr4krlws0k12g21phhjwy4z40000gn/T/prysk-tests-1oo0sdqw/verbosity.t",
    "heap": null,
    "login": null,
    "no_color": false,
    "preflight": false,
    "team": null,
    "token": null,
    "trace": null,
    "verbosity": 2,
    "check_for_update": false,
    "test_run": false,
    "run_args": null,
    "command": {
      "Run": {
        "cache_dir": null,
        "cache_workers": 10,
        "concurrency": null,
        "continue_execution": false,
        "dry_run": null,
        "single_package": false,
        "filter": [
          "util"
        ],
        "force": true,
        "global_deps": [],
        "graph": null,
        "ignore": [],
        "include_dependencies": false,
        "no_cache": false,
        "no_daemon": false,
        "no_deps": false,
        "output_logs": null,
        "only": false,
        "parallel": false,
        "pkg_inference_root": "",
        "profile": null,
        "remote_only": false,
        "scope": [],
        "since": null,
        "tasks": [
          "build"
        ],
        "pass_through_args": []
      }
    }
  }
  2023-02-13T20:19:15.239+0000 [DEBUG] turbo: Found go binary at "/Users/mehulkar/dev/vercel/turbo/target/debug/go-turbo"
  2023-02-13T20:19:15.246Z [INFO]  turbo: skipping turbod since we appear to be in a non-interactive context
  2023-02-13T20:19:15.247Z [DEBUG] turbo: global hash env vars: vars=["VERCEL_ANALYTICS_ID"]
  2023-02-13T20:19:15.247Z [DEBUG] turbo: global hash: value=430b3790556340cb
  2023-02-13T20:19:15.247Z [DEBUG] turbo: local cache folder: path=""
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  2023-02-13T20:19:15.270Z [DEBUG] turbo.: start
  2023-02-13T20:19:15.270Z [DEBUG] turbo: task hash env vars for util:build: vars=[]
  2023-02-13T20:19:15.270Z [DEBUG] turbo: task hash: value=6dec18f9f767112f
  util:build: cache bypass, force executing 6dec18f9f767112f
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  2023-02-13T20:19:15.472Z [DEBUG] turbo.: caching output: outputs="{[packages/util/.turbo/turbo-build.log] []}"
  2023-02-13T20:19:15.472Z [DEBUG] turbo.: done: status=complete duration=201.79325ms
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:    226ms 
  
 


Make sure users can only use one verbosity flag
  $ ${TURBO} build -v --verbosity=1
  ERROR the argument '-v...' cannot be used with '--verbosity <COUNT>'
  
  Usage: turbo [OPTIONS] [COMMAND]
  
  For more information, try '--help'.
  
  [1]
