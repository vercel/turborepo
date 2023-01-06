(() => {
  // When a chunk is executed, it will either register itself with the current
  // instance of the runtime, or it will push itself onto the list of pending
  // chunks (`self.TURBOPACK`).
  //
  // When the runtime executes, it will pick up and register all pending chunks,
  // and replace the list of pending chunks with itself so later chunks can
  // register directly with it.

  /* eslint-disable @next/next/no-assign-module-variable */

  if (!Array.isArray(self.TURBOPACK)) {
    return;
  }

  /** @typedef {import('../types').ChunkRegistration} ChunkRegistration */
  /** @typedef {import('../types').ChunkModule} ChunkModule */
  /** @typedef {import('../types').Chunk} Chunk */
  /** @typedef {import('../types').ModuleFactory} ModuleFactory */

  /** @typedef {import('../types').ChunkPath} ChunkPath */
  /** @typedef {import('../types').ModuleId} ModuleId */

  /** @typedef {import('../types').Module} Module */
  /** @typedef {import('../types').Exports} Exports */
  /** @typedef {import('../types').EsmInteropNamespace} EsmInteropNamespace */
  /** @typedef {import('../types').Runnable} Runnable */

  /** @typedef {import('../types').Runtime} Runtime */

  /** @typedef {import('../types/runtime').Loader} Loader */

  /** @type {ChunkRegistration[]} */
  const chunksToRegister = self.TURBOPACK;
  /** @type {Array<Runnable>} */
  let runnable = [];
  /** @type {Object.<ModuleId, ModuleFactory>} */
  const moduleFactories = { __proto__: null };
  /** @type {Object.<ModuleId, Module>} */
  const moduleCache = { __proto__: null };
  /**
   * Contains the IDs of all chunks that have been loaded.
   *
   * @type {Set<ChunkPath>}
   */
  const loadedChunks = new Set();
  /**
   * Maps a chunk ID to the chunk's loader if the chunk is currently being loaded.
   *
   * @type {Map<ChunkPath, Loader>}
   */
  const chunkLoaders = new Map();
  /**
   * Module IDs that are instantiated as part of the runtime of a chunk.
   *
   * @type {Set<ModuleId>}
   */
  const runtimeModules = new Set();
  /**
   * Map from module ID to the chunks that contain this module.
   *
   * In HMR, we need to keep track of which modules are contained in which so
   * chunks. This is so we don't eagerly dispose of a module when it is removed
   * from chunk A, but still exists in chunk B.
   *
   * @type {Map<ModuleId, Set<string>>}
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

  /**
   * @param {ModuleId} from
   * @param {string} chunkPath
   * @returns {Promise<any> | undefined}
   */
  function loadChunk(from, chunkPath) {
    if (loadedChunks.has(chunkPath)) {
      return Promise.resolve();
    }

    const chunkLoader = getOrCreateChunkLoader(chunkPath, from);

    return chunkLoader.promise;
  }

  /**
   * @param {string} chunkPath
   * @param {ModuleId} from
   * @returns {Loader}
   */
  function getOrCreateChunkLoader(chunkPath, from) {
    let chunkLoader = chunkLoaders.get(chunkPath);
    if (chunkLoader) {
      return chunkLoader;
    }

    let resolve;
    let reject;
    const promise = new Promise((innerResolve, innerReject) => {
      resolve = innerResolve;
      reject = innerReject;
    });

    const onError = (msg) => {
      chunkLoaders.delete(chunkPath);
      reject(new Error(`Failed to load chunk from ${chunkPath}: ${msg}`));
    };

    const onLoad = () => {
      chunkLoaders.delete(chunkPath);
      resolve();
    };

    chunkLoader = {
      promise,
      onLoad,
    };
    chunkLoaders.set(chunkPath, chunkLoader);

    const moduleChunkPaths = moduleChunksMap.get(from);
    if (moduleChunkPaths == null) {
      onError(
        `Module ${from} that requested chunk ${chunkPath} has been removed`
      );
      return;
    }

    const fromPath = moduleChunkPaths.values().next().value;
    const path = require("path");
    const resolved = require.resolve(
      "./" + path.relative(path.dirname(fromPath), chunkPath)
    );
    delete require.cache[resolved];
    require(resolved);

    loadedChunks.add(chunkPath);
    onLoad();

    return chunkLoader;
  }

  /**
   * @enum {number}
   */
  const SourceType = {
    /**
     * The module was instantiated because it was included in an evaluated chunk's
     * runtime.
     */
    Runtime: 0,
    /**
     * The module was instantiated because a parent module imported it.
     */
    Parent: 1,
  };

  /**
   *
   * @param {ModuleId} id
   * @param {SourceType} sourceType
   * @param {ModuleId} [sourceId]
   * @returns {Module}
   */
  function instantiateModule(id, sourceType, sourceId) {
    const moduleFactory = moduleFactories[id];
    if (typeof moduleFactory !== "function") {
      // This can happen if modules incorrectly handle HMR disposes/updates,
      // e.g. when they keep a `setTimeout` around which still executes old code
      // and contains e.g. a `require("something")` call.
      let instantiationReason;
      switch (sourceType) {
        case SourceType.Runtime:
          instantiationReason = "as a runtime entry";
          break;
        case SourceType.Parent:
          instantiationReason = `because it was required from module ${sourceId}`;
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
      parents: [],
      children: [],
      interopNamespace: undefined,
    };
    moduleCache[id] = module;

    if (sourceType === SourceType.Runtime) {
      runtimeModules.add(id);
    } else if (sourceType === SourceType.Parent) {
      module.parents.push(sourceId);

      // No need to add this module as a child of the parent module here, this
      // has already been taken care of in `getOrInstantiateModuleFromParent`.
    }

    moduleFactory.call(module.exports, {
      e: module.exports,
      r: commonJsRequire.bind(null, module),
      x: externalRequire,
      i: esmImport.bind(null, module),
      s: esm.bind(null, module.exports),
      v: exportValue.bind(null, module),
      m: module,
      c: moduleCache,
      l: loadChunk.bind(null, id),
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

    return instantiateModule(id, SourceType.Parent, sourceModule.id);
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
   */
  /**
   *
   * @param {ModuleId} moduleId
   * @returns {Module}
   */
  function instantiateRuntimeModule(moduleId) {
    return instantiateModule(moduleId, SourceType.Runtime);
  }

  function markChunkAsLoaded(chunkPath) {
    loadedChunks.add(chunkPath);

    const chunkLoader = chunkLoaders.get(chunkPath);
    if (!chunkLoader) {
      // This happens for all initial chunks that are loaded directly from
      // the HTML.
      return;
    }

    // Only chunks that are loaded via `loadChunk` will have a loader.
    chunkLoader.onLoad();
  }

  /** @type {Runtime} */
  const runtime = {
    loadedChunks,
    modules: moduleFactories,
    cache: moduleCache,
    instantiateRuntimeModule,
  };

  /**
   * @param {ChunkRegistration} chunkRegistration
   */
  function registerChunk([chunkPath, chunkModules, ...run]) {
    markChunkAsLoaded(chunkPath);
    for (const [moduleId, moduleFactory] of Object.entries(chunkModules)) {
      if (!moduleFactories[moduleId]) {
        moduleFactories[moduleId] = moduleFactory;
      }
      addModuleToChunk(moduleId, chunkPath);
    }
    runnable.push(...run);
    runnable = runnable.filter((r) => r(runtime));
  }

  self.TURBOPACK = { push: registerChunk };
  chunksToRegister.forEach(registerChunk);
})();
