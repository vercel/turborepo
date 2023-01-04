Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  
  Tasks to Run
  build
    Task            = build                  
    Hash            = c7223f212c321d3b       
    Cached (Local)  = false                  
    Cached (Remote) = false                  
    Command         = echo 'building'        
    Outputs         =                        
    Log File        = .turbo/turbo-build.log 
    Dependencies    =                        
    Dependendents   =                        

  $ ${TURBO} run build --dry=json --single-package
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
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
        "dependents": []
      }
    ]
  }
