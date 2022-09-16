Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task          = build                  
    Hash          = 9f3c4fb1ea7d561d       
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
        "hash": "9f3c4fb1ea7d561d",
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
