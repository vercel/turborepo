Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Global Hash Inputs
    Global Files               = 2
    External Dependencies Hash = 
    Global Cache Key           = Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo
    Root pipeline              = {"//#build":{"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false}}
  
  Tasks to Run
  build
    Task                   = build                                                                                                   
    Hash                   = c7223f212c321d3b                                                                                        
    Cached (Local)         = false                                                                                                   
    Cached (Remote)        = false                                                                                                   
    Command                = echo 'building'                                                                                         
    Outputs                =                                                                                                         
    Log File               = .turbo/turbo-build.log                                                                                  
    Dependencies           =                                                                                                         
    Dependendents          =                                                                                                         
    ResolvedTaskDefinition = {"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 

  $ ${TURBO} run build --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "c7223f212c321d3b",
        "cacheState": {
          "local": false,
          "remote": false
        },
        "command": "echo 'building'",
        "outputs": null,
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [],
          "cache": false,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        }
      }
    ]
  }
