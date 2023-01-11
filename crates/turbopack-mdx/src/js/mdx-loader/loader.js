/**
 * @typedef {import('vfile').VFileCompatible} VFileCompatible
 * @typedef {import('vfile').VFile} VFile
 * @typedef {import('vfile-message').VFileMessage} VFileMessage
 * @typedef {import('@mdx-js/mdx').CompileOptions} CompileOptions
 * @typedef {Pick<CompileOptions, 'SourceMapGenerator'>} Defaults
 * @typedef {Omit<CompileOptions, 'SourceMapGenerator'>} Options
 * @typedef {import('webpack').LoaderContext<unknown>} LoaderContext
 * @typedef {import('webpack').Compiler} WebpackCompiler
 * @typedef {(vfileCompatible: VFileCompatible) => Promise<VFile>} Process
 */

import { createHash } from "node:crypto";
import path from "node:path";
import { SourceMapGenerator } from "source-map";
import { createFormatAwareProcessors } from "@mdx-js/mdx/lib/util/create-format-aware-processors.js";

const own = {}.hasOwnProperty;

// Note: the cache is heavily inspired by:
// <https://github.com/TypeStrong/ts-loader/blob/5c030bf/src/instance-cache.ts>
const marker = /** @type {WebpackCompiler} */ ({});
/** @type {WeakMap<WebpackCompiler, Map<string, Process>>} */
const cache = new WeakMap();

/**
 * A Webpack (5+) loader for MDX.
 * See `webpack.cjs`, which wraps this, because Webpack loaders must currently
 * be CommonJS.
 *
 * @this {LoaderContext}
 * @param {string} value
 * @param {(error: Error|null|undefined, content?: string|Buffer, map?: Object) => void} callback
 */
export function loader(value, callback) {
  /** @type {Defaults} */
  const defaults = this.sourceMap ? { SourceMapGenerator } : {};
  const options = {
    development: this.mode === "development",
    .../** @type {CompileOptions} */ (this.getOptions()),
  };
  const config = { ...defaults, ...options };
  const hash = getOptionsHash(options);
  // Some loaders set `undefined` (see `TypeStrong/ts-loader`).
  /* c8 ignore next */
  const compiler = this._compiler || marker;

  /* Removed option. */
  /* c8 ignore next 5 */
  if ("renderer" in config) {
    throw new Error(
      "`options.renderer` is no longer supported. Please see <https://mdxjs.com/migrating/v2/> for more information"
    );
  }

  let map = cache.get(compiler);

  if (!map) {
    map = new Map();
    cache.set(compiler, map);
  }

  let process = map.get(hash);

  if (!process) {
    process = createFormatAwareProcessors(config).process;
    map.set(hash, process);
  }

  process({ value, path: this.resourcePath }).then(
    (file) => {
      callback(null, file.value, file.map);
    },
    (/** @type VFileMessage */ error) => {
      const fpath = path.relative(this.context, this.resourcePath);
      error.message = `${fpath}:${error.name}: ${error.message}`;
      callback(error);
    }
  );
}

/**
 * @param {Options} options
 */
function getOptionsHash(options) {
  const hash = createHash("sha256");
  /** @type {keyof Options} */
  let key;

  for (key in options) {
    if (own.call(options, key)) {
      const value = options[key];

      if (value !== undefined) {
        const valueString = JSON.stringify(value);
        hash.update(key + valueString);
      }
    }
  }

  return hash.digest("hex").slice(0, 16);
}
