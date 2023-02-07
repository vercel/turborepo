Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task                   = build                                                                                                       
    Hash                   = 7bf32e1dedb04a5d                                                                                            
    Cached (Local)         = false                                                                                                       
    Cached (Remote)        = false                                                                                                       
    Command                = echo 'building' > foo                                                                                       
    Outputs                = foo                                                                                                         
    Log File               = .turbo/turbo-build.log                                                                                      
    Dependencies           =                                                                                                             
    Dependendents          =                                                                                                             
    ResolvedTaskDefinition = {"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 

  $ ${TURBO} run build --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "7bf32e1dedb04a5d",
        "cacheState": {
          "local": false,
          "remote": false
        },
        "command": "echo 'building' \u003e foo",
        "outputs": [
          "foo"
        ],
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [
            "foo"
          ],
          "cache": true,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        }
      }
    ]
  }
