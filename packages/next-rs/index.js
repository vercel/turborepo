const path = require("path");
const { platform, arch } = require("os");
const { platformArchTriples } = require("@napi-rs/triples");

const ArchName = arch();
const PlatformName = platform();
const triples = platformArchTriples[PlatformName][ArchName] || [];

// Allow to specify an absolute path to the custom turbopack binary to load.
// If one of env variables is set, `loadNative` will try to use any turbo-* interfaces from specified
// binary instead. This will not affect existing swc's transform, or other interfaces. This is thin,
// naive interface - `loadBindings` will not validate neither path nor the binary.
//
// Note these are internal flag: there's no stability, feature guarantee.
const __INTERNAL_CUSTOM_TURBOPACK_BINARY =
  process.env.__INTERNAL_CUSTOM_TURBOPACK_BINARY;
const __INTERNAL_CUSTOM_TURBOPACK_BINDINGS =
  process.env.__INTERNAL_CUSTOM_TURBOPACK_BINDINGS;

let nativeBindings;
let wasmBindings;

function toBuffer(t) {
  return Buffer.from(JSON.stringify(t));
}

exports.loadWasm = async function loadWasm(importPath = "") {
  if (wasmBindings) {
    return wasmBindings;
  }

  let attempts = [];
  for (let pkg of ["@next/rs-wasm-nodejs", "@next/rs-wasm-web"]) {
    try {
      let pkgPath = pkg;

      if (importPath) {
        // the import path must be exact when not in node_modules
        pkgPath = path.join(importPath, pkg, "wasm.js");
      }
      let bindings = await import(pkgPath);
      if (pkg === "@next/rs-wasm-web") {
        bindings = await bindings.default();
      }
      //Log.info('Using wasm build of next-rs')

      // Note wasm binary does not support async intefaces yet, all async
      // interface coereces to sync interfaces.
      wasmBindings = {
        isWasm: true,
        transform(src, options) {
          // TODO: we can remove fallback to sync interface once new stable version of next-rs gets published (current v12.2)
          return bindings?.transform
            ? bindings.transform(src.toString(), options)
            : Promise.resolve(bindings.transformSync(src.toString(), options));
        },
        transformSync(src, options) {
          return bindings.transformSync(src.toString(), options);
        },
        minify(src, options) {
          return bindings?.minify
            ? bindings.minify(src.toString(), options)
            : Promise.resolve(bindings.minifySync(src.toString(), options));
        },
        minifySync(src, options) {
          return bindings.minifySync(src.toString(), options);
        },
        parse(src, options) {
          return bindings?.parse
            ? bindings.parse(src.toString(), options)
            : Promise.resolve(bindings.parseSync(src.toString(), options));
        },
        parseSync(src, options) {
          const astStr = bindings.parseSync(src.toString(), options);
          return astStr;
        },
        getTargetTriple() {
          return undefined;
        },
        turbo: {
          startDev: () => {
            //Log.error('Wasm binding does not support --turbo yet')
          },
          startTrace: () => {
            //Log.error('Wasm binding does not support trace yet')
          },
        },
        mdx: {
          compile: (src, options) => bindings.mdxCompile(src, options),
          compileSync: (src, options) => bindings.mdxCompileSync(src, options),
        },
      };
      return wasmBindings;
    } catch (e) {
      // Only log attempts for loading wasm when loading as fallback
      if (importPath) {
        if (e?.code === "ERR_MODULE_NOT_FOUND") {
          attempts.push(`Attempted to load ${pkg}, but it was not installed`);
        } else {
          attempts.push(
            `Attempted to load ${pkg}, but an error occurred: ${e.message ?? e}`
          );
        }
      }
    }
  }

  throw attempts;
};

exports.loadNative = function loadNative(isCustomTurbopack = false) {
  if (nativeBindings) {
    return nativeBindings;
  }

  let bindings;
  let attempts = [];

  for (const triple of triples) {
    try {
      bindings = require(`@next/rs/native/next-rs.${triple.platformArchABI}.node`);
      //Log.info('Using locally built binary of @next/rs')
      break;
    } catch (e) {}
  }

  if (!bindings) {
    for (const triple of triples) {
      let pkg = `@next/rs-${triple.platformArchABI}`;
      try {
        bindings = require(pkg);
        break;
      } catch (e) {
        if (e?.code === "MODULE_NOT_FOUND") {
          attempts.push(`Attempted to load ${pkg}, but it was not installed`);
        } else {
          attempts.push(
            `Attempted to load ${pkg}, but an error occurred: ${e.message ?? e}`
          );
        }
      }
    }
  }

  if (bindings) {
    nativeBindings = {
      isWasm: false,
      transform(src, options) {
        const isModule =
          typeof src !== undefined &&
          typeof src !== "string" &&
          !Buffer.isBuffer(src);
        options = options || {};

        if (options?.jsc?.parser) {
          options.jsc.parser.syntax = options.jsc.parser.syntax ?? "ecmascript";
        }

        return bindings.transform(
          isModule ? JSON.stringify(src) : src,
          isModule,
          toBuffer(options)
        );
      },

      transformSync(src, options) {
        if (typeof src === undefined) {
          throw new Error(
            "transformSync doesn't implement reading the file from filesystem"
          );
        } else if (Buffer.isBuffer(src)) {
          throw new Error(
            "transformSync doesn't implement taking the source code as Buffer"
          );
        }
        const isModule = typeof src !== "string";
        options = options || {};

        if (options?.jsc?.parser) {
          options.jsc.parser.syntax = options.jsc.parser.syntax ?? "ecmascript";
        }

        return bindings.transformSync(
          isModule ? JSON.stringify(src) : src,
          isModule,
          toBuffer(options)
        );
      },

      minify(src, options) {
        return bindings.minify(toBuffer(src), toBuffer(options ?? {}));
      },

      minifySync(src, options) {
        return bindings.minifySync(toBuffer(src), toBuffer(options ?? {}));
      },

      parse(src, options) {
        return bindings.parse(src, toBuffer(options ?? {}));
      },

      getTargetTriple: bindings.getTargetTriple,
      initCustomTraceSubscriber: bindings.initCustomTraceSubscriber,
      teardownTraceSubscriber: bindings.teardownTraceSubscriber,
      teardownCrashReporter: bindings.teardownCrashReporter,
      turbo: {
        startDev: (options) => {
          const devOptions = {
            ...options,
            noOpen: options.noOpen ?? true,
          };

          if (!isCustomTurbopack) {
            bindings.startTurboDev(toBuffer(devOptions));
          } else if (!!__INTERNAL_CUSTOM_TURBOPACK_BINARY) {
            console.warn(
              `Loading custom turbopack binary from ${__INTERNAL_CUSTOM_TURBOPACK_BINARY}`
            );

            return new Promise((resolve, reject) => {
              const spawn = require("next/dist/compiled/cross-spawn");
              const args = [];

              Object.entries(devOptions).forEach(([key, value]) => {
                let cli_key = `--${key.replace(
                  /[A-Z]/g,
                  (m) => "-" + m.toLowerCase()
                )}`;
                if (key === "dir") {
                  args.push(value);
                } else if (typeof value === "boolean" && value === true) {
                  args.push(cli_key);
                } else if (typeof value !== "boolean" && !!value) {
                  args.push(cli_key, value);
                }
              });

              console.warn(`Running turbopack with args: [${args.join(" ")}]`);

              const child = spawn(__INTERNAL_CUSTOM_TURBOPACK_BINARY, args, {
                stdio: "inherit",
                env: {
                  ...process.env,
                },
              });
              child.on("message", (message) => {
                console.log(message);
              });
              child.on("close", (code) => {
                if (code !== 0) {
                  reject({
                    command: `${__INTERNAL_CUSTOM_TURBOPACK_BINARY} ${args.join(
                      " "
                    )}`,
                  });
                  return;
                }
                resolve(0);
              });
            });
          } else if (!!__INTERNAL_CUSTOM_TURBOPACK_BINDINGS) {
            console.warn(
              `Loading custom turbopack bindings from ${__INTERNAL_CUSTOM_TURBOPACK_BINARY}`
            );
            console.warn(`Running turbopack with args: `, devOptions);

            require(__INTERNAL_CUSTOM_TURBOPACK_BINDINGS).startDev(devOptions);
          }
        },
        startTrace: (options = {}, turboTasks) =>
          bindings.runTurboTracing(
            toBuffer({ exact: true, ...options }),
            turboTasks
          ),
        createTurboTasks: (memoryLimit) =>
          bindings.createTurboTasks(memoryLimit),
      },
      mdx: {
        compile: (src, options) =>
          bindings.mdxCompile(src, toBuffer(options ?? {})),
        compileSync: (src, options) =>
          bindings.mdxCompileSync(src, toBuffer(options ?? {})),
      },
    };
    return nativeBindings;
  }

  throw attempts;
};
