Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/query

  $ ${TURBO} query "query { package(name: \"app-a\") { tasks { items { name } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "name": "build"
            },
            {
              "name": "custom"
            },
            {
              "name": "test"
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"app-a\") { tasks { items { fullName directDependencies { items { fullName } } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "fullName": "app-a#build",
              "directDependencies": {
                "items": []
              }
            },
            {
              "fullName": "app-a#custom",
              "directDependencies": {
                "items": []
              }
            },
            {
              "fullName": "app-a#test",
              "directDependencies": {
                "items": [
                  {
                    "fullName": "app-a#prepare"
                  },
                  {
                    "fullName": "lib-a#build0"
                  }
                ]
              }
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"lib-b\") { tasks { items { fullName directDependents { items { fullName } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "fullName": "lib-b#build",
              "directDependents": {
                "items": []
              }
            },
            {
              "fullName": "lib-b#build0",
              "directDependents": {
                "items": [
                  {
                    "fullName": "app-b#build0"
                  },
                  {
                    "fullName": "app-b#test"
                  },
                  {
                    "fullName": "lib-a#build0"
                  },
                  {
                    "fullName": "lib-a#test"
                  }
                ]
              }
            },
            {
              "fullName": "lib-b#test",
              "directDependents": {
                "items": []
              }
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"lib-b\") { tasks { items { fullName allDependents { items { fullName } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "fullName": "lib-b#build",
              "allDependents": {
                "items": []
              }
            },
            {
              "fullName": "lib-b#build0",
              "allDependents": {
                "items": [
                  {
                    "fullName": "app-a#build0"
                  },
                  {
                    "fullName": "app-a#test"
                  },
                  {
                    "fullName": "app-b#build0"
                  },
                  {
                    "fullName": "app-b#test"
                  },
                  {
                    "fullName": "lib-a#build0"
                  },
                  {
                    "fullName": "lib-a#test"
                  }
                ]
              }
            },
            {
              "fullName": "lib-b#test",
              "allDependents": {
                "items": []
              }
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"app-a\") { tasks { items { fullName allDependencies { items { fullName } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "fullName": "app-a#build",
              "allDependencies": {
                "items": []
              }
            },
            {
              "fullName": "app-a#custom",
              "allDependencies": {
                "items": []
              }
            },
            {
              "fullName": "app-a#test",
              "allDependencies": {
                "items": [
                  {
                    "fullName": "app-a#prepare"
                  },
                  {
                    "fullName": "lib-a#build0"
                  },
                  {
                    "fullName": "lib-a#prepare"
                  },
                  {
                    "fullName": "lib-b#build0"
                  },
                  {
                    "fullName": "lib-b#prepare"
                  },
                  {
                    "fullName": "lib-d#build0"
                  },
                  {
                    "fullName": "lib-d#prepare"
                  }
                ]
              }
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"lib-b\") { tasks { items { fullName indirectDependents { items { fullName } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "fullName": "lib-b#build",
              "indirectDependents": {
                "items": []
              }
            },
            {
              "fullName": "lib-b#build0",
              "indirectDependents": {
                "items": [
                  {
                    "fullName": "app-a#build0"
                  },
                  {
                    "fullName": "app-a#test"
                  },
                  {
                    "fullName": "app-b#build0"
                  },
                  {
                    "fullName": "app-b#test"
                  },
                  {
                    "fullName": "lib-a#build0"
                  },
                  {
                    "fullName": "lib-a#test"
                  }
                ]
              }
            },
            {
              "fullName": "lib-b#test",
              "indirectDependents": {
                "items": []
              }
            }
          ]
        }
      }
    }
  }

  $ ${TURBO} query "query { package(name: \"app-a\") { tasks { items { fullName indirectDependencies { items { fullName } } } } } }" | jq
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "package": {
        "tasks": {
          "items": [
            {
              "fullName": "app-a#build",
              "indirectDependencies": {
                "items": []
              }
            },
            {
              "fullName": "app-a#custom",
              "indirectDependencies": {
                "items": []
              }
            },
            {
              "fullName": "app-a#test",
              "indirectDependencies": {
                "items": [
                  {
                    "fullName": "lib-a#prepare"
                  },
                  {
                    "fullName": "lib-b#build0"
                  },
                  {
                    "fullName": "lib-b#prepare"
                  },
                  {
                    "fullName": "lib-d#build0"
                  },
                  {
                    "fullName": "lib-d#prepare"
                  }
                ]
              }
            }
          ]
        }
      }
    }
  }
