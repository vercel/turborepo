Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task          = build                  
    Hash          = 3e7a2ac81b9d11be       
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
        "hash": "3e7a2ac81b9d11be",
        "command": "echo 'building'",
        "outputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": []
      }
    ]
  }
