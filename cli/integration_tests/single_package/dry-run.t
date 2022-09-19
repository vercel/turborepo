Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task          = build                  
    Hash          = e491d0044f4b9b90       
    Directory     =                        
    Command       = echo 'building' > foo  
    Outputs       = foo                    
    Log File      = .turbo/turbo-build.log 
    Dependencies  =                        
    Dependendents =                        

  $ ${TURBO} run build --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "e491d0044f4b9b90",
        "command": "echo 'building' \u003e foo",
        "outputs": [
          "foo"
        ],
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": []
      }
    ]
  }
