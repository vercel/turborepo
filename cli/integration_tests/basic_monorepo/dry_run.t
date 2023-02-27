Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

# Run the build task with --dry flag and cut up the logs into separate files by empty lines
# https://stackoverflow.com/a/33297878/986415
  $ ${TURBO} run build --dry |  awk -v RS= '{print > ("tmp-" NR ".txt")}'

# The first part of the file is Packages in Scope
  $ cat tmp-1.txt
  Packages in Scope
  Name   Path          
  my-app apps/my-app   
  util   packages/util 

# Part 2 of the logs are Global Hash INputs
  $ cat tmp-2.txt
  Global Hash Inputs
    Global Files               = 1
    External Dependencies Hash = ccab0b28617f1f56
    Global Cache Key           = Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo
    Root pipeline              = {"build":{"outputs":[],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false},"my-app#build":{"outputs":["apple.json","banana.txt"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false}}

# Part 3 are Tasks to Run, and we have to validate each task separately
  $ cat tmp-3.txt | grep "my-app#build" -A 12
  my-app#build
    Task                   = build                                                                                                                           
    Package                = my-app                                                                                                                          
    Hash                   = 8888a278aaecb070                                                                                                                
    Cached (Local)         = false                                                                                                                           
    Cached (Remote)        = false                                                                                                                           
    Directory              = apps/my-app                                                                                                                     
    Command                = echo 'building'                                                                                                                 
    Outputs                = apple.json, banana.txt                                                                                                          
    Log File               = apps/my-app/.turbo/turbo-build.log                                                                                              
    Dependencies           =                                                                                                                                 
    Dependendents          =                                                                                                                                 
    ResolvedTaskDefinition = {"outputs":["apple.json","banana.txt"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 

  $ cat tmp-3.txt | grep "util#build" -A 12
  util#build
    Task                   = build                                                                                                  
    Package                = util                                                                                                   
    Hash                   = d09a52ea72495c87                                                                                       
    Cached (Local)         = false                                                                                                  
    Cached (Remote)        = false                                                                                                  
    Directory              = packages/util                                                                                          
    Command                = echo 'building'                                                                                        
    Outputs                =                                                                                                        
    Log File               = packages/util/.turbo/turbo-build.log                                                                   
    Dependencies           =                                                                                                        
    Dependendents          =                                                                                                        
    ResolvedTaskDefinition = {"outputs":[],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 

# Save JSON to tmp file so we don't need to keep re-running the build
  $ ${TURBO} run build --dry=json > tmpjson.log

  $ cat tmpjson.log | jq .globalHashSummary
  {
    "globalFileHashMap": {
      "foo.txt": "eebae5f3ca7b5831e429e947b7d61edd0de69236"
    },
    "rootExternalDepsHash": "ccab0b28617f1f56",
    "globalCacheKey": "Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo",
    "pipeline": {
      "build": {
        "outputs": [],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [],
        "persistent": false
      },
      "my-app#build": {
        "outputs": [
          "apple.json",
          "banana.txt"
        ],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [],
        "persistent": false
      }
    }
  }

# Validate output of my-app#build task
  $ cat tmpjson.log | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0]'
  {
    "taskId": "my-app#build",
    "task": "build",
    "package": "my-app",
    "hash": "8888a278aaecb070",
    "cacheState": {
      "local": false,
      "remote": false
    },
    "command": "echo 'building'",
    "outputs": [
      "apple.json",
      "banana.txt"
    ],
    "excludedOutputs": null,
    "logFile": "apps/my-app/.turbo/turbo-build.log",
    "directory": "apps/my-app",
    "dependencies": [],
    "dependents": [],
    "resolvedTaskDefinition": {
      "outputs": [
        "apple.json",
        "banana.txt"
      ],
      "cache": true,
      "dependsOn": [],
      "inputs": [],
      "outputMode": "full",
      "env": [],
      "persistent": false
    }
  }

# Validate output of util#build task
  $ cat tmpjson.log | jq '.tasks | map(select(.taskId == "util#build")) | .[0]'
  {
    "taskId": "util#build",
    "task": "build",
    "package": "util",
    "hash": "d09a52ea72495c87",
    "cacheState": {
      "local": false,
      "remote": false
    },
    "command": "echo 'building'",
    "outputs": null,
    "excludedOutputs": null,
    "logFile": "packages/util/.turbo/turbo-build.log",
    "directory": "packages/util",
    "dependencies": [],
    "dependents": [],
    "resolvedTaskDefinition": {
      "outputs": [],
      "cache": true,
      "dependsOn": [],
      "inputs": [],
      "outputMode": "full",
      "env": [],
      "persistent": false
    }
  }

Tasks that don't exist throw an error
  $ ${TURBO} run doesnotexist --dry=json
   ERROR  run failed: error preparing engine: Could not find the following tasks in project: doesnotexist
  Turbo error: error preparing engine: Could not find the following tasks in project: doesnotexist
  [1]
