Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task          = build                  
    Hash          = 1c6df0e48c4a821d       
    Directory     =                        
    Command       = echo 'building'        
    Outputs       =                        
    Log File      = .turbo/turbo-build.log 
    Dependencies  =                        
    Dependendents =                        

  $ ${TURBO} run build --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "1c6df0e48c4a821d",
        "command": "echo 'building'",
        "outputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": []
      }
    ]
  }
