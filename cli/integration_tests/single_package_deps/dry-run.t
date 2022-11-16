Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${SHIM} run test --dry --single-package
  
  Tasks to Run
  build
    Task            = build                  
    Hash            = 8fc80cfff3b64237       
    Cached (Local)  = false                  
    Cached (Remote) = false                  
    Command         = echo 'building' > foo  
    Outputs         = foo                    
    Log File        = .turbo/turbo-build.log 
    Dependencies    =                        
    Dependendents   = test                   
  test
    Task            = test                                         
    Hash            = c71366ccd6a86465                             
    Cached (Local)  = false                                        
    Cached (Remote) = false                                        
    Command         = [[ ( -f foo ) && $(cat foo) == 'building' ]] 
    Outputs         =                                              
    Log File        = .turbo/turbo-test.log                        
    Dependencies    = build                                        
    Dependendents   =                                              

  $ ${SHIM} run test --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "8fc80cfff3b64237",
        "command": "echo 'building' \u003e foo",
        "outputs": [
          "foo"
        ],
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [
          "test"
        ]
      },
      {
        "task": "test",
        "hash": "c71366ccd6a86465",
        "command": "[[ ( -f foo ) \u0026\u0026 $(cat foo) == 'building' ]]",
        "outputs": null,
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-test.log",
        "dependencies": [
          "build"
        ],
        "dependents": []
      }
    ]
  }
