/* eslint-disable @next/next/no-assign-module-variable */

/** @typedef {import('../types').ModuleFactory} ModuleFactory */

/** @typedef {import('../types').ChunkPath} ChunkPath */
/** @typedef {import('../types').ModuleId} ModuleId */

/** @typedef {import('../types').Module} Module */
/** @typedef {import('../types').SourceInfo} SourceInfo */
/** @typedef {import('../types').SourceType} SourceType */
/** @typedef {import('../types').SourceType.Runtime} SourceTypeRuntime */
/** @typedef {import('../types').SourceType.Parent} SourceTypeParent */
/** @typedef {import('../types').Exports} Exports */
/** @typedef {import('../types').EsmInteropNamespace} EsmInteropNamespace */

const path = require("path");
const relativePathToRuntimeRoot = path.relative(RUNTIME_PUBLIC_PATH, ".");
const RUNTIME_ROOT = path.resolve(__filename, relativePathToRuntimeRoot);

/** @type {Object.<ModuleId, ModuleFactory>} */
const moduleFactories = { __proto__: null };
/** @type {Object.<ModuleId, Module>} */
const moduleCache = { __proto__: null };
/**
 * Module IDs that are instantiated as part of the runtime of a chunk.
 *
 * @type {Set<ModuleId>}
 */
const runtimeModules = new Set();
/**
 * Map from module ID to the chunks that contain this module.
 *
 * @type {Map<ModuleId, Set<ChunkPath>>}
 */
const moduleChunksMap = new Map();

const hOP = Object.prototype.hasOwnProperty;
const _process =
  typeof process !== "undefined"
    ? process
    : {
        env: {},
        // Some modules rely on `process.browser` to execute browser-specific code.
        // NOTE: `process.browser` is specific to Webpack.
        browser: true,
      };

const toStringTag = typeof Symbol !== "undefined" && Symbol.toStringTag;

/**
 * @param {any} obj
 * @param {PropertyKey} name
 * @param {PropertyDescriptor & ThisType<any>} options
 */
function defineProp(obj, name, options) {
  if (!hOP.call(obj, name)) Object.defineProperty(obj, name, options);
}

/**
 * Adds the getters to the exports object
 *
 * @param {Exports} exports
 * @param {Record<string, () => any>} getters
 */
function esm(exports, getters) {
  defineProp(exports, "__esModule", { value: true });
  if (toStringTag) defineProp(exports, toStringTag, { value: "Module" });
  for (const key in getters) {
    defineProp(exports, key, { get: getters[key], enumerable: true });
  }
}

/**
 * Adds the getters to the exports object
 *
 * @param {Exports} exports
 * @param {Record<string, any>} props
 */
function cjs(exports, props) {
  for (const key in props) {
    defineProp(exports, key, { get: () => props[key], enumerable: true });
  }
}

/**
 * @param {Module} module
 * @param {any} value
 */
function exportValue(module, value) {
  module.exports = value;
}

/**
 * @param {Record<string, any>} obj
 * @param {string} key
 */
function createGetter(obj, key) {
  return () => obj[key];
}

/**
 * @param {Exports} raw
 * @param {EsmInteropNamespace} ns
 * @param {boolean} [allowExportDefault]
 */
function interopEsm(raw, ns, allowExportDefault) {
  /** @type {Object.<string, () => any>} */
  const getters = { __proto__: null };
  for (const key in raw) {
    getters[key] = createGetter(raw, key);
  }
  if (!(allowExportDefault && "default" in getters)) {
    getters["default"] = () => raw;
  }
  esm(ns, getters);
}

/**
 * @param {Module} sourceModule
 * @param {ModuleId} id
 * @param {boolean} allowExportDefault
 * @returns {EsmInteropNamespace}
 */
function esmImport(sourceModule, id, allowExportDefault) {
  const module = getOrInstantiateModuleFromParent(id, sourceModule);
  const raw = module.exports;
  if (raw.__esModule) return raw;
  if (module.interopNamespace) return module.interopNamespace;
  const ns = (module.interopNamespace = {});
  interopEsm(raw, ns, allowExportDefault);
  return ns;
}

/**
 * @param {Module} sourceModule
 * @param {ModuleId} id
 * @returns {Exports}
 */
function commonJsRequire(sourceModule, id) {
  return getOrInstantiateModuleFromParent(id, sourceModule).exports;
}

function externalRequire(id, esm) {
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
  const ns = {};
  interopEsm(raw, ns, true);
  return ns;
}
externalRequire.resolve = (name, opt) => {
  return require.resolve(name, opt);
};

/**
 * @param {ChunkPath} chunkPath
 */
function loadChunk(chunkPath) {
  if (!chunkPath.endsWith(".js")) {
    // We only support loading JS chunks in Node.js.
    // This branch can be hit when trying to load a CSS chunk.
    return;
  }

  const path = require("path");
  const resolved = require.resolve(path.resolve(RUNTIME_ROOT, chunkPath));
  delete require.cache[resolved];
  const chunkModules = require(resolved);

  for (const [moduleId, moduleFactory] of Object.entries(chunkModules)) {
    if (!moduleFactories[moduleId]) {
      moduleFactories[moduleId] = moduleFactory;
    }
    addModuleToChunk(moduleId, chunkPath);
  }
}

/**
 * @param {SourceInfo} source
 * @param {string} chunkPath
 * @returns {Promise<void>}
 */
function loadChunkAsync(source, chunkPath) {
  return new Promise((resolve, reject) => {
    try {
      loadChunk(chunkPath);
    } catch (err) {
      reject(err);
      return;
    }
    resolve();
  });
}

/** @type {SourceTypeRuntime} */
const SourceTypeRuntime = 0;
/** @type {SourceTypeParent} */
const SourceTypeParent = 1;

/**
 *
 * @param {ModuleId} id
 * @param {SourceInfo} source
 * @returns {Module}
 */
function instantiateModule(id, source) {
  const moduleFactory = moduleFactories[id];
  if (typeof moduleFactory !== "function") {
    // This can happen if modules incorrectly handle HMR disposes/updates,
    // e.g. when they keep a `setTimeout` around which still executes old code
    // and contains e.g. a `require("something")` call.
    let instantiationReason;
    switch (source.type) {
      case SourceTypeRuntime:
        instantiationReason = `as a runtime entry of chunk ${source.chunkPath}`;
        break;
      case SourceTypeParent:
        instantiationReason = `because it was required from module ${source.parentId}`;
        break;
    }
    throw new Error(
      `Module ${id} was instantiated ${instantiationReason}, but the module factory is not available. It might have been deleted in an HMR update.`
    );
  }

  /** @type {Module} */
  const module = {
    exports: {},
    loaded: false,
    id,
    parents: undefined,
    children: [],
    interopNamespace: undefined,
  };
  moduleCache[id] = module;

  switch (source.type) {
    case SourceTypeRuntime:
      runtimeModules.add(id);
      module.parents = [];
      break;
    case SourceTypeParent:
      // No need to add this module as a child of the parent module here, this
      // has already been taken care of in `getOrInstantiateModuleFromParent`.
      module.parents = [source.parentId];
      break;
  }

  moduleFactory.call(module.exports, {
    e: module.exports,
    r: commonJsRequire.bind(null, module),
    x: externalRequire,
    i: esmImport.bind(null, module),
    s: esm.bind(null, module.exports),
    j: cjs.bind(null, module.exports),
    v: exportValue.bind(null, module),
    m: module,
    c: moduleCache,
    l: loadChunk.bind(null, { type: SourceTypeParent, parentId: id }),
    p: _process,
    g: globalThis,
    __dirname: module.id.replace(/(^|\/)[\/]+$/, ""),
  });

  module.loaded = true;
  if (module.interopNamespace) {
    // in case of a circular dependency: cjs1 -> esm2 -> cjs1
    interopEsm(module.exports, module.interopNamespace);
  }

  return module;
}

/**
 * Retrieves a module from the cache, or instantiate it if it is not cached.
 *
 * @param {ModuleId} id
 * @param {Module} sourceModule
 * @returns {Module}
 */
function getOrInstantiateModuleFromParent(id, sourceModule) {
  const module = moduleCache[id];

  if (sourceModule.children.indexOf(id) === -1) {
    sourceModule.children.push(id);
  }

  if (module) {
    if (module.parents.indexOf(sourceModule.id) === -1) {
      module.parents.push(sourceModule.id);
    }

    return module;
  }

  return instantiateModule(id, {
    type: SourceTypeParent,
    parentId: sourceModule.id,
  });
}

/**
 * Adds a module to a chunk.
 *
 * @param {ModuleId} moduleId
 * @param {ChunkPath} chunkPath
 */
function addModuleToChunk(moduleId, chunkPath) {
  let moduleChunks = moduleChunksMap.get(moduleId);
  if (!moduleChunks) {
    moduleChunks = new Set([chunkPath]);
    moduleChunksMap.set(moduleId, moduleChunks);
  } else {
    moduleChunks.add(chunkPath);
  }
}

/**
 * Instantiates a runtime module.
 *
 * @param {ModuleId} moduleId
 * @param {ChunkPath} chunkPath
 * @returns {Module}
 */
function instantiateRuntimeModule(moduleId, chunkPath) {
  return instantiateModule(moduleId, { type: SourceTypeRuntime, chunkPath });
}

/**
 * Retrieves a module from the cache, or instantiate it as a runtime module if it is not cached.
 *
 * @param {ModuleId} moduleId
 * @param {ChunkPath} chunkPath
 * @returns {Module}
 */
function getOrInstantiateRuntimeModule(moduleId, chunkPath) {
  const module = moduleCache[moduleId];

  if (module) {
    return module;
  }

  return instantiateRuntimeModule(moduleId, chunkPath);
}

module.exports = {
  getOrInstantiateRuntimeModule,
  loadChunk,
};
