Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh turbo_trace

  $ ${TURBO} query "query { file(path: \"main.ts\") { path } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "main.ts"
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"main.ts\") { path, dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "main.ts",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "bar.js"
              },
              {
                "path": "button.css"
              },
              {
                "path": "button.tsx"
              },
              {
                "path": "foo.js"
              },
              {
                "path": "node_modules(\/|\\\\)repeat-string(\/|\\\\)index.js" (re)
              }
            ]
          }
        }
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"button.tsx\") { path, dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "button.tsx",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "button.css"
              }
            ]
          }
        }
      }
    }
  }

  $ ${TURBO} query "query { file(path: \"circular.ts\") { path dependencies { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "circular.ts",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "circular2.ts"
              }
            ]
          }
        }
      }
    }
  }

Trace file with invalid import
  $ ${TURBO} query "query { file(path: \"invalid.ts\") { path dependencies { files { items { path } } errors { items { message } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "invalid.ts",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "button.css"
              },
              {
                "path": "button.tsx"
              }
            ]
          },
          "errors": {
            "items": [
              {
                "message": "failed to resolve import to `./non-existent-file.js`"
              }
            ]
          }
        }
      }
    }
  }

Get AST from file
  $ ${TURBO} query "query { file(path: \"main.ts\") { path ast } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "main.ts",
        "ast": {
          "type": "Module",
          "span": {
            "start": 1,
            "end": 169
          },
          "body": [
            {
              "type": "ImportDeclaration",
              "span": {
                "start": 1,
                "end": 35
              },
              "specifiers": [
                {
                  "type": "ImportSpecifier",
                  "span": {
                    "start": 10,
                    "end": 16
                  },
                  "local": {
                    "type": "Identifier",
                    "span": {
                      "start": 10,
                      "end": 16
                    },
                    "ctxt": 0,
                    "value": "Button",
                    "optional": false
                  },
                  "imported": null,
                  "isTypeOnly": false
                }
              ],
              "source": {
                "type": "StringLiteral",
                "span": {
                  "start": 24,
                  "end": 34
                },
                "value": "./button",
                "raw": "\"./button\""
              },
              "typeOnly": false,
              "with": null,
              "phase": "evaluation"
            },
            {
              "type": "ImportDeclaration",
              "span": {
                "start": 36,
                "end": 60
              },
              "specifiers": [
                {
                  "type": "ImportDefaultSpecifier",
                  "span": {
                    "start": 43,
                    "end": 46
                  },
                  "local": {
                    "type": "Identifier",
                    "span": {
                      "start": 43,
                      "end": 46
                    },
                    "ctxt": 0,
                    "value": "foo",
                    "optional": false
                  }
                }
              ],
              "source": {
                "type": "StringLiteral",
                "span": {
                  "start": 52,
                  "end": 59
                },
                "value": "./foo",
                "raw": "\"./foo\""
              },
              "typeOnly": false,
              "with": null,
              "phase": "evaluation"
            },
            {
              "type": "ImportDeclaration",
              "span": {
                "start": 61,
                "end": 96
              },
              "specifiers": [
                {
                  "type": "ImportDefaultSpecifier",
                  "span": {
                    "start": 68,
                    "end": 74
                  },
                  "local": {
                    "type": "Identifier",
                    "span": {
                      "start": 68,
                      "end": 74
                    },
                    "ctxt": 0,
                    "value": "repeat",
                    "optional": false
                  }
                }
              ],
              "source": {
                "type": "StringLiteral",
                "span": {
                  "start": 80,
                  "end": 95
                },
                "value": "repeat-string",
                "raw": "\"repeat-string\""
              },
              "typeOnly": false,
              "with": null,
              "phase": "evaluation"
            },
            {
              "type": "VariableDeclaration",
              "span": {
                "start": 98,
                "end": 126
              },
              "ctxt": 0,
              "kind": "const",
              "declare": false,
              "declarations": [
                {
                  "type": "VariableDeclarator",
                  "span": {
                    "start": 104,
                    "end": 125
                  },
                  "id": {
                    "type": "Identifier",
                    "span": {
                      "start": 104,
                      "end": 110
                    },
                    "ctxt": 0,
                    "value": "button",
                    "optional": false,
                    "typeAnnotation": null
                  },
                  "init": {
                    "type": "NewExpression",
                    "span": {
                      "start": 113,
                      "end": 125
                    },
                    "ctxt": 0,
                    "callee": {
                      "type": "Identifier",
                      "span": {
                        "start": 117,
                        "end": 123
                      },
                      "ctxt": 0,
                      "value": "Button",
                      "optional": false
                    },
                    "arguments": [],
                    "typeArguments": null
                  },
                  "definite": false
                }
              ]
            },
            {
              "type": "ExpressionStatement",
              "span": {
                "start": 128,
                "end": 144
              },
              "expression": {
                "type": "CallExpression",
                "span": {
                  "start": 128,
                  "end": 143
                },
                "ctxt": 0,
                "callee": {
                  "type": "MemberExpression",
                  "span": {
                    "start": 128,
                    "end": 141
                  },
                  "object": {
                    "type": "Identifier",
                    "span": {
                      "start": 128,
                      "end": 134
                    },
                    "ctxt": 0,
                    "value": "button",
                    "optional": false
                  },
                  "property": {
                    "type": "Identifier",
                    "span": {
                      "start": 135,
                      "end": 141
                    },
                    "value": "render"
                  }
                },
                "arguments": [],
                "typeArguments": null
              }
            },
            {
              "type": "ExpressionStatement",
              "span": {
                "start": 145,
                "end": 162
              },
              "expression": {
                "type": "CallExpression",
                "span": {
                  "start": 145,
                  "end": 161
                },
                "ctxt": 0,
                "callee": {
                  "type": "Identifier",
                  "span": {
                    "start": 145,
                    "end": 151
                  },
                  "ctxt": 0,
                  "value": "repeat",
                  "optional": false
                },
                "arguments": [
                  {
                    "spread": null,
                    "expression": {
                      "type": "StringLiteral",
                      "span": {
                        "start": 152,
                        "end": 157
                      },
                      "value": "foo",
                      "raw": "\"foo\""
                    }
                  },
                  {
                    "spread": null,
                    "expression": {
                      "type": "NumericLiteral",
                      "span": {
                        "start": 159,
                        "end": 160
                      },
                      "value": 5.0,
                      "raw": "5"
                    }
                  }
                ],
                "typeArguments": null
              }
            },
            {
              "type": "ExpressionStatement",
              "span": {
                "start": 163,
                "end": 169
              },
              "expression": {
                "type": "CallExpression",
                "span": {
                  "start": 163,
                  "end": 168
                },
                "ctxt": 0,
                "callee": {
                  "type": "Identifier",
                  "span": {
                    "start": 163,
                    "end": 166
                  },
                  "ctxt": 0,
                  "value": "foo",
                  "optional": false
                },
                "arguments": [],
                "typeArguments": null
              }
            }
          ],
          "interpreter": null
        }
      }
    }
  }

Set depth for dependencies
  $ ${TURBO} query "query { file(path: \"main.ts\") { path dependencies(depth: 1) { files { items { path } } } } }"
   WARNING  query command is experimental and may change in the future
  {
    "data": {
      "file": {
        "path": "main.ts",
        "dependencies": {
          "files": {
            "items": [
              {
                "path": "button.tsx"
              },
              {
                "path": "foo.js"
              },
              {
                "path": "node_modules(\/|\\\\)repeat-string(\/|\\\\)index.js" (re)
              }
            ]
          }
        }
      }
    }
  }