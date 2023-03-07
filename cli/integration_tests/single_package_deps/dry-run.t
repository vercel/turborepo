Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run test --dry --single-package
  
  Global Hash Inputs
    Global Files               = 2
    External Dependencies Hash = 
    Global Cache Key           = Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo
    Root pipeline              = {"//#build":{"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false},"//#test":{"outputs":[],"cache":true,"dependsOn":["build"],"inputs":[],"outputMode":"full","env":[],"persistent":false}}
  
  Tasks to Run
  build
    Task                             = build                                                                                                       
    Hash                             = 8fc80cfff3b64237                                                                                            
    Cached (Local)                   = false                                                                                                       
    Cached (Remote)                  = false                                                                                                       
    Command                          = echo 'building' > foo                                                                                       
    Outputs                          = foo                                                                                                         
    Log File                         = .turbo/turbo-build.log                                                                                      
    Dependencies                     =                                                                                                             
    Dependendents                    = test                                                                                                        
    Inputs Files Considered          = 4                                                                                                           
    Configured Environment Variables =                                                                                                             
    Inferred Environment Variables   =                                                                                                             
    Global Environment Variables     = VERCEL_ANALYTICS_ID=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855                        
    ResolvedTaskDefinition           = {"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 
    Framework                        = <NO FRAMEWORK DETECTED>                                                                                     
  test
    Task                             = test                                                                                                          
    Hash                             = c71366ccd6a86465                                                                                              
    Cached (Local)                   = false                                                                                                         
    Cached (Remote)                  = false                                                                                                         
    Command                          = [[ ( -f foo ) && $(cat foo) == 'building' ]]                                                                  
    Outputs                          =                                                                                                               
    Log File                         = .turbo/turbo-test.log                                                                                         
    Dependencies                     = build                                                                                                         
    Dependendents                    =                                                                                                               
    Inputs Files Considered          = 4                                                                                                             
    Configured Environment Variables =                                                                                                               
    Inferred Environment Variables   =                                                                                                               
    Global Environment Variables     = VERCEL_ANALYTICS_ID=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855                          
    ResolvedTaskDefinition           = {"outputs":[],"cache":true,"dependsOn":["build"],"inputs":[],"outputMode":"full","env":[],"persistent":false} 
    Framework                        = <NO FRAMEWORK DETECTED>                                                                                       

  $ ${TURBO} run test --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "8fc80cfff3b64237",
        "cacheState": {
          "local": false,
          "remote": false
        },
        "command": "echo 'building' \u003e foo",
        "outputs": [
          "foo"
        ],
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [
          "test"
        ],
        "resolvedTaskDefinition": {
          "outputs": [
            "foo"
          ],
          "cache": true,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        },
        "expandedInputs": {
          ".gitignore": "6f23ff6842b5526da43ab38f4a5bf3b0158eeb42",
          "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
          "package.json": "bc24e5c5b8bd13d419e0742ae3e92a2bf61c53d0",
          "turbo.json": "e1fe3e5402fe019ef3845cc63a736878a68934c7"
        },
        "expandedOutputs": null,
        "framework": "\u003cNO FRAMEWORK DETECTED\u003e",
        "environmentVariables": {
          "configured": [],
          "inferred": [],
          "global": [
            "VERCEL_ANALYTICS_ID=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
          ]
        }
      },
      {
        "task": "test",
        "hash": "c71366ccd6a86465",
        "cacheState": {
          "local": false,
          "remote": false
        },
        "command": "[[ ( -f foo ) \u0026\u0026 $(cat foo) == 'building' ]]",
        "outputs": null,
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-test.log",
        "dependencies": [
          "build"
        ],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [],
          "cache": true,
          "dependsOn": [
            "build"
          ],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        },
        "expandedInputs": {
          ".gitignore": "6f23ff6842b5526da43ab38f4a5bf3b0158eeb42",
          "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
          "package.json": "bc24e5c5b8bd13d419e0742ae3e92a2bf61c53d0",
          "turbo.json": "e1fe3e5402fe019ef3845cc63a736878a68934c7"
        },
        "expandedOutputs": null,
        "framework": "\u003cNO FRAMEWORK DETECTED\u003e",
        "environmentVariables": {
          "configured": [],
          "inferred": [],
          "global": [
            "VERCEL_ANALYTICS_ID=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
          ]
        }
      }
    ]
  }
