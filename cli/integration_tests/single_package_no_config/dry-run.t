Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task          = build                  
    Hash          = c207d64157b1635a       
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
        "hash": "c207d64157b1635a",
        "command": "echo 'building'",
        "outputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": []
      }
    ]
  }
