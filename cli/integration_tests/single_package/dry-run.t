Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${SHIM} run build --dry --single-package
  
  Tasks to Run
  build
    Task            = build                  
    Hash            = 7bf32e1dedb04a5d       
    Cached (Local)  = false                  
    Cached (Remote) = false                  
    Command         = echo 'building' > foo  
    Outputs         = foo                    
    Log File        = .turbo/turbo-build.log 
    Dependencies    =                        
    Dependendents   =                        

  $ ${SHIM} run build --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "7bf32e1dedb04a5d",
        "command": "echo 'building' \u003e foo",
        "outputs": [
          "foo"
        ],
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": []
      }
    ]
  }
