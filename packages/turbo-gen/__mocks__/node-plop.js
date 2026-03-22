const path = require("node:path");

const configHandlers = new Map();

function toBundledPath(configPath) {
  return configPath.replace(/\.(ts|js|cjs|mts|mjs)$/, ".turbo-gen-bundled.cjs");
}

function resolveConfigHandler(configPath) {
  if (configHandlers.has(configPath)) {
    return configHandlers.get(configPath);
  }

  if (configPath.endsWith(".turbo-gen-bundled.cjs")) {
    for (const ext of [".ts", ".js", ".cjs", ".mts", ".mjs"]) {
      const originalPath = configPath.replace(".turbo-gen-bundled.cjs", ext);
      if (configHandlers.has(originalPath)) {
        return configHandlers.get(originalPath);
      }
    }
  }

  return undefined;
}

function nodePlop(configPath, options = {}) {
  const generators = new Map();
  let activeDestBasePath = options.destBasePath ?? process.cwd();

  const api = {
    getGeneratorList() {
      return [...generators.values()].map((generator) => ({
        name: generator.name,
        description: generator.description
      }));
    },
    getGenerator(name) {
      return generators.get(name);
    },
    async load(targetConfigPath, loadOptions = {}) {
      const previousDestBasePath = activeDestBasePath;
      activeDestBasePath = loadOptions.destBasePath ?? previousDestBasePath;
      const handler = resolveConfigHandler(targetConfigPath);
      if (handler) {
        await handler(api);
      }
      activeDestBasePath = previousDestBasePath;
    },
    setGenerator(name, generatorConfig = {}) {
      const generator = {
        name,
        description: generatorConfig.description ?? "",
        basePath: path.join(activeDestBasePath, "turbo", "generators"),
        runPrompts: async () => ({}),
        runActions: async () => ({
          changes: [],
          failures: []
        })
      };
      generators.set(name, generator);
      return generator;
    }
  };

  if (configPath) {
    const handler = resolveConfigHandler(configPath);
    if (handler) {
      handler(api);
    }
  }

  return api;
}

nodePlop.__setConfig = (configPath, handler) => {
  configHandlers.set(configPath, handler);
  configHandlers.set(toBundledPath(configPath), handler);
};

nodePlop.__reset = () => {
  configHandlers.clear();
};

module.exports = nodePlop;
module.exports.default = nodePlop;
