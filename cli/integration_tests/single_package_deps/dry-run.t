Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run test --dry --single-package
  
  Tasks to Run
  build
    Task          = build                  
    Hash          = 3f50a33cc496d697       
    Directory     =                        
    Command       = echo 'building' > foo  
    Outputs       = foo                    
    Log File      = .turbo/turbo-build.log 
    Dependencies  =                        
    Dependendents = test                   
  test
    Task          = test                                         
    Hash          = d5990f5197c8e71b                             
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
        "hash": "3f50a33cc496d697",
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
        "hash": "d5990f5197c8e71b",
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
