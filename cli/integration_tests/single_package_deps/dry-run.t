Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run test --dry --single-package
  
  Tasks to Run
  build
    Task          = build                  
    Hash          = fb5ab7cab2c98c77       
    Directory     =                        
    Command       = echo 'building' > foo  
    Outputs       = foo                    
    Log File      = .turbo/turbo-build.log 
    Dependencies  =                        
    Dependendents = test                   
  test
    Task          = test                                         
    Hash          = 3d586528c591ec52                             
    Directory     =                                              
    Command       = [[ ( -f foo ) && $(cat foo) == 'building' ]] 
    Outputs       =                                              
    Log File      = .turbo/turbo-test.log                        
    Dependencies  = build                                        
    Dependendents =                                              

  $ ${TURBO} run test --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "fb5ab7cab2c98c77",
        "command": "echo 'building' \u003e foo",
        "outputs": [
          "foo"
        ],
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [
          "test"
        ]
      },
      {
        "task": "test",
        "hash": "3d586528c591ec52",
        "command": "[[ ( -f foo ) \u0026\u0026 $(cat foo) == 'building' ]]",
        "outputs": [],
        "logFile": ".turbo/turbo-test.log",
        "dependencies": [
          "build"
        ],
        "dependents": []
      }
    ]
  }
