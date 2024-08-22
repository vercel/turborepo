Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Run info
  $ ${TURBO} ls
   WARNING  ls command is experimental and may change in the future
  3 packages (npm)
  
    another packages[\/\\]another (re)
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)

Run info with json output
  $ ${TURBO} ls --output=json
   WARNING  ls command is experimental and may change in the future
  {
    "packageManager": "npm",
    "packages": {
      "count": 3,
      "items": [
        {
          "name": "another",
          "path": "packages(\/|\\\\)another" (re)
        },
        {
          "name": "my-app",
          "path": "apps(\/|\\\\)my-app" (re)
        },
        {
          "name": "util",
          "path": "packages(\/|\\\\)util" (re)
        }
      ]
    }
  }

Run info with filter
  $ ${TURBO} ls -F my-app...
   WARNING  ls command is experimental and may change in the future
  2 packages (npm)
  
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)

Run info on package `another`
  $ ${TURBO} ls another
   WARNING  ls command is experimental and may change in the future
  another depends on: <no packages>
  
  tasks: <no tasks>
  

Run info on package `my-app`
  $ ${TURBO} ls my-app
   WARNING  ls command is experimental and may change in the future
  my-app depends on: util
  
  tasks:
    build: echo building
    maybefails: exit 4
  
Run info on package `my-app` with json output
  $ ${TURBO} ls my-app --output=json
   WARNING  ls command is experimental and may change in the future
  {
    "packages": [
      {
        "name": "my-app",
        "tasks": {
          "count": 2,
          "items": [
            {
              "name": "build",
              "command": "echo building"
            },
            {
              "name": "maybefails",
              "command": "exit 4"
            }
          ]
        },
        "dependencies": [
          "util"
        ]
      }
    ]
  }
