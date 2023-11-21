/// <reference path="../shared/runtime-utils.ts" />

declare var RUNTIME_PUBLIC_PATH: string;
declare var OUTPUT_ROOT: string;
declare var ASSET_PREFIX: string;

const path = require("path");

const relativePathToRuntimeRoot = path.relative(RUNTIME_PUBLIC_PATH, ".");
// Compute the relative path to the `distDir`.
const relativePathToDistRoot = path.relative(
  path.join(OUTPUT_ROOT, RUNTIME_PUBLIC_PATH),
  "."
);
const RUNTIME_ROOT = path.resolve(__filename, relativePathToRuntimeRoot);
// Compute the absolute path to the root, by stripping distDir from the absolute path to this file.
const ABSOLUTE_ROOT = path.resolve(__filename, relativePathToDistRoot);

interface RequireContextEntry {
  external: boolean;
}

function commonJsRequireContext(
  entry: RequireContextEntry,
  sourceModule: Module
): Exports {
  return entry.external
    ? externalRequire(entry.id(), false)
    : commonJsRequire(sourceModule, entry.id());
}

async function externalImport(id: ModuleId) {
  let raw;
  try {
    raw = await import(id);
  } catch (err) {
    // TODO(alexkirsz) This can happen when a client-side module tries to load
    // an external module we don't provide a shim for (e.g. querystring, url).
    // For now, we fail semi-silently, but in the future this should be a
    // compilation error.
    throw new Error(`Failed to load external module ${id}: ${err}`);
  }

  if (raw && raw.__esModule && raw.default && "default" in raw.default) {
    return interopEsm(raw.default, {}, true);
  }

  return raw;
}

function externalRequire(
  id: ModuleId,
  esm: boolean = false
): Exports | EsmNamespaceObject {
  let raw;
  try {
    raw = require(id);
  } catch (err) {
    // TODO(alexkirsz) This can happen when a client-side module tries to load
    // an external module we don't provide a shim for (e.g. querystring, url).
    // For now, we fail semi-silently, but in the future this should be a
    // compilation error.
    throw new Error(`Failed to load external module ${id}: ${err}`);
  }

  if (!esm || raw.__esModule) {
    return raw;
  }

  return interopEsm(raw, {}, true);
}

externalRequire.resolve = (
  id: string,
  options?: {
    paths?: string[];
  }
) => {
  return require.resolve(id, options);
};

/**
 * Returns an absolute path to the given module path.
 * Module path should be relative, either path to a file or a directory.
 *
 * This fn allows to calculate an absolute path for some global static values, such as
 * `__dirname` or `import.meta.url` that Turbopack will not embeds in compile time.
 * See ImportMetaBinding::code_generation for the usage.
 */
function resolveAbsolutePath(modulePath?: string): string {
  if (modulePath) {
    // Module path can contain common relative path to the root, recalaute to avoid duplicated joined path.
    const relativePathToRoot = path.relative(ABSOLUTE_ROOT, modulePath);
    return path.join(ABSOLUTE_ROOT, relativePathToRoot);
  }
  return ABSOLUTE_ROOT;
}
