Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task            = build                  
    Hash            = fcdc2cfedba63a1f       
    Cached (Local)  = false                  
    Cached (Remote) = false                  
    Command         = echo 'building'        
    Outputs         =                        
    Log File        = .turbo/turbo-build.log 
    Dependencies    =                        
    Dependendents   =                        

  $ ${TURBO} run build --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "fcdc2cfedba63a1f",
        "command": "echo 'building'",
        "outputs": null,
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": []
      }
    ]
  }
