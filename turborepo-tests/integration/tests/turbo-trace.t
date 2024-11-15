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
                "path": "button.json"
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
              },
              {
                "path": "button.json"
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
  $ ${TURBO} query "query { file(path: \"invalid.ts\") { path dependencies { files { items { path } } errors { items { message } } } } }" 2>/dev/null
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
                "path": "button.json"
              },
              {
                "path": "button.tsx"
              }
            ]
          },
          "errors": {
            "items": [
              {
                "message": "failed to resolve import to `./non-existent-file.js` in `.*`" (re)
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
            "end": 173
          },
          "body": [
            {
              "type": "ImportDeclaration",
              "span": {
                "start": 1,
                "end": 39
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
                  "end": 38
                },
                "value": "./button.tsx",
                "raw": "\"./button.tsx\""
              },
              "typeOnly": false,
              "with": null,
              "phase": "evaluation"
            },
            {
              "type": "ImportDeclaration",
              "span": {
                "start": 40,
                "end": 64
              },
              "specifiers": [
                {
                  "type": "ImportDefaultSpecifier",
                  "span": {
                    "start": 47,
                    "end": 50
                  },
                  "local": {
                    "type": "Identifier",
                    "span": {
                      "start": 47,
                      "end": 50
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
                  "start": 56,
                  "end": 63
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
                "start": 65,
                "end": 100
              },
              "specifiers": [
                {
                  "type": "ImportDefaultSpecifier",
                  "span": {
                    "start": 72,
                    "end": 78
                  },
                  "local": {
                    "type": "Identifier",
                    "span": {
                      "start": 72,
                      "end": 78
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
                  "start": 84,
                  "end": 99
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
                "start": 102,
                "end": 130
              },
              "ctxt": 0,
              "kind": "const",
              "declare": false,
              "declarations": [
                {
                  "type": "VariableDeclarator",
                  "span": {
                    "start": 108,
                    "end": 129
                  },
                  "id": {
                    "type": "Identifier",
                    "span": {
                      "start": 108,
                      "end": 114
                    },
                    "ctxt": 0,
                    "value": "button",
                    "optional": false,
                    "typeAnnotation": null
                  },
                  "init": {
                    "type": "NewExpression",
                    "span": {
                      "start": 117,
                      "end": 129
                    },
                    "ctxt": 0,
                    "callee": {
                      "type": "Identifier",
                      "span": {
                        "start": 121,
                        "end": 127
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
                "start": 132,
                "end": 148
              },
              "expression": {
                "type": "CallExpression",
                "span": {
                  "start": 132,
                  "end": 147
                },
                "ctxt": 0,
                "callee": {
                  "type": "MemberExpression",
                  "span": {
                    "start": 132,
                    "end": 145
                  },
                  "object": {
                    "type": "Identifier",
                    "span": {
                      "start": 132,
                      "end": 138
                    },
                    "ctxt": 0,
                    "value": "button",
                    "optional": false
                  },
                  "property": {
                    "type": "Identifier",
                    "span": {
                      "start": 139,
                      "end": 145
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
                "start": 149,
                "end": 166
              },
              "expression": {
                "type": "CallExpression",
                "span": {
                  "start": 149,
                  "end": 165
                },
                "ctxt": 0,
                "callee": {
                  "type": "Identifier",
                  "span": {
                    "start": 149,
                    "end": 155
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
                        "start": 156,
                        "end": 161
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
                        "start": 163,
                        "end": 164
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
                "start": 167,
                "end": 173
              },
              "expression": {
                "type": "CallExpression",
                "span": {
                  "start": 167,
                  "end": 172
                },
                "ctxt": 0,
                "callee": {
                  "type": "Identifier",
                  "span": {
                    "start": 167,
                    "end": 170
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