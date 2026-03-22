import path from "node:path";
import { createRequire } from "node:module";
import fs from "fs-extra";
import type { Project } from "@turbo/workspaces";
import type { NodePlopAPI, PlopGenerator } from "node-plop";
import nodePlop from "node-plop";
import { Separator } from "@inquirer/prompts";
import { searchUp, getTurboConfigs, logger } from "@turbo/utils";
import { build as esbuild } from "esbuild";
import { GeneratorError } from "./error";

const SUPPORTED_CONFIG_EXTENSIONS = ["ts", "js", "cjs", "mts", "mjs"];
const TURBO_GENERATOR_DIRECTORY = path.join("turbo", "generators");

const SUPPORTED_WORKSPACE_GENERATOR_CONFIGS = SUPPORTED_CONFIG_EXTENSIONS.map(
  (ext) => path.join(TURBO_GENERATOR_DIRECTORY, `config.${ext}`)
);

const SUPPORTED_ROOT_GENERATOR_CONFIGS = [
  ...SUPPORTED_WORKSPACE_GENERATOR_CONFIGS,
  ...SUPPORTED_CONFIG_EXTENSIONS.map((ext) => path.join(`plopfile.${ext}`))
];

export type Generator = PlopGenerator & {
  basePath: string;
  name: string;
};

export async function getPlop({
  project,
  configPath
}: {
  project: Project;
  configPath?: string;
}): Promise<NodePlopAPI | undefined> {
  const workspaceConfigs = getWorkspaceGeneratorConfigs({ project });
  let plop: NodePlopAPI | undefined;

  try {
    if (configPath) {
      if (!fs.existsSync(configPath)) {
        throw new GeneratorError(`No config at "${configPath}"`, {
          type: "plop_no_config"
        });
      }

      try {
        plop = await nodePlop(await bundleConfigForLoading(configPath), {
          destBasePath: configPath,
          force: false
        });
      } catch (e) {
        logger.error(e);
      }
    } else {
      for (const possiblePath of SUPPORTED_ROOT_GENERATOR_CONFIGS) {
        const plopFile = path.join(project.paths.root, possiblePath);
        if (!fs.existsSync(plopFile)) {
          continue;
        }

        try {
          plop = await nodePlop(await bundleConfigForLoading(plopFile), {
            destBasePath: project.paths.root,
            force: false
          });
          break;
        } catch (e) {
          logger.error(e);
        }
      }

      if (!plop && workspaceConfigs.length > 0) {
        plop = await nodePlop(
          await bundleConfigForLoading(workspaceConfigs[0].config),
          {
            destBasePath: workspaceConfigs[0].root,
            force: false
          }
        );
        workspaceConfigs.shift();
      }
    }

    if (plop) {
      for (const c of workspaceConfigs) {
        try {
          await plop.load(await bundleConfigForLoading(c.config), {
            destBasePath: c.root,
            force: false
          });
        } catch (e) {
          logger.error(e);
        }
      }
    }
  } finally {
    cleanupBundledConfigs();
  }

  return plop;
}

export async function getCustomGenerators({
  project,
  configPath
}: {
  project: Project;
  configPath?: string;
}): Promise<Array<Generator | InstanceType<typeof Separator>>> {
  const plop = await getPlop({ project, configPath });

  if (!plop) {
    return [];
  }

  const gens = plop.getGeneratorList();
  const gensWithDetails = gens.map((g) => plop.getGenerator(g.name));

  const gensByWorkspace: Record<string, Array<Generator>> = {};
  for (const g of gensWithDetails) {
    const generatorDetails = g as Generator;
    const gensWorkspace = project.workspaceData.workspaces.find((w) => {
      if (generatorDetails.basePath === project.paths.root) {
        return false;
      }
      const parts = generatorDetails.basePath.split(path.sep);
      parts.pop();
      parts.pop();
      const workspaceRoot = path.join("/", ...parts);
      return workspaceRoot === w.paths.root;
    });

    if (gensWorkspace) {
      if (!(gensWorkspace.name in gensByWorkspace)) {
        gensByWorkspace[gensWorkspace.name] = [];
      }
      gensByWorkspace[gensWorkspace.name].push(generatorDetails);
    } else {
      if (!("root" in gensByWorkspace)) {
        gensByWorkspace.root = [];
      }
      gensByWorkspace.root.push(generatorDetails);
    }
  }

  const gensWithSeparators: Array<Generator | InstanceType<typeof Separator>> =
    [];
  for (const group of Object.keys(gensByWorkspace)) {
    gensWithSeparators.push(new Separator(group));
    gensWithSeparators.push(...gensByWorkspace[group]);
  }

  return gensWithSeparators;
}

export async function getCustomGenerator({
  project,
  generator,
  configPath
}: {
  project: Project;
  generator: string;
  configPath?: string;
}): Promise<string | undefined> {
  const plop = await getPlop({ project, configPath });
  if (!plop) {
    return undefined;
  }

  try {
    const gen = plop.getGenerator(generator) as PlopGenerator | undefined;
    if (gen) {
      return generator;
    }
    return undefined;
  } catch (e) {
    return undefined;
  }
}

function injectTurborepoData({
  project,
  generator
}: {
  project: Project;
  generator: PlopGenerator & { basePath?: string };
}) {
  const paths = {
    cwd: process.cwd(),
    root: project.paths.root,
    workspace: generator.basePath
      ? searchUp({ cwd: generator.basePath, target: "package.json" })
      : undefined
  };
  let turboConfigs = {};
  try {
    turboConfigs = getTurboConfigs(generator.basePath);
  } catch (e) {
    // ignore
  }

  return {
    turbo: {
      paths,
      configs: turboConfigs
    }
  };
}

// node-plop uses require() to load config files, which can't handle TypeScript
// or ESM syntax. We use esbuild at runtime to bundle the user's config into a
// single CJS file before node-plop loads it. esbuild handles TS transpilation
// and ESM-to-CJS conversion transparently.
const bundled = new Set<string>();

// Modules provided by @turbo/gen that user configs may import without
// installing themselves (backward compat). When esbuild can't resolve these
// from the user's project, we resolve them from @turbo/gen's own node_modules.
const CLI_PROVIDED_MODULES = ["@inquirer/prompts"];

// Resolve the directory containing @turbo/gen's own node_modules so we can
// use it as a fallback resolution path for CLI-provided modules.
function getOwnNodeModulesDirs(): Array<string> {
  const dirs: Array<string> = [];
  try {
    const ownRequire = createRequire(__filename);
    const ownPkgPath = ownRequire.resolve("@inquirer/prompts");
    // Walk up from the resolved path to find the node_modules directory
    let dir = path.dirname(ownPkgPath);
    while (dir !== path.dirname(dir)) {
      if (path.basename(dir) === "node_modules") {
        dirs.push(dir);
        break;
      }
      dir = path.dirname(dir);
    }
  } catch {
    // If we can't resolve our own deps, the fallback won't work but
    // the user's own node_modules might still have what's needed.
  }
  return dirs;
}

async function bundleConfigForLoading(configPath: string): Promise<string> {
  const outName = path
    .basename(configPath)
    .replace(/\.(ts|js|cjs|mts|mjs)$/, ".turbo-gen-bundled.cjs");
  const outDir = path.dirname(configPath);
  const outPath = path.join(outDir, outName);

  try {
    const ownNodeModules = getOwnNodeModulesDirs();

    const result = await esbuild({
      entryPoints: [configPath],
      outfile: outPath,
      bundle: true,
      format: "cjs",
      platform: "node",
      // Fallback resolution: if a bare specifier isn't found in the user's
      // project, check @turbo/gen's own node_modules (for @inquirer/prompts
      // and other CLI-provided modules).
      nodePaths: ownNodeModules,
      plugins: [cliProvidedModulesPlugin(configPath)],
      logLevel: "silent",
      // node-plop loads config files via `await import()`. For CJS files,
      // Node wraps module.exports as the default export. esbuild's CJS output
      // for ESM sources produces `{ __esModule: true, default: fn }`, which
      // causes double-wrapping: `{ default: { __esModule: true, default: fn } }`.
      // This footer unwraps the __esModule pattern so module.exports is the
      // function itself, making `import().default` resolve correctly.
      footer: {
        js: `if(module.exports&&module.exports.__esModule&&typeof module.exports.default==="function"){module.exports=module.exports.default;}`
      }
    });

    if (result.errors.length > 0) {
      return configPath;
    }

    bundled.add(outPath);
    return outPath;
  } catch {
    return configPath;
  }
}

// esbuild plugin: for CLI-provided modules that can't be resolved from the
// user's project, try resolving from @turbo/gen's own dependency tree.
// This maintains backward compat for configs that import @inquirer/prompts
// without installing it themselves.
function cliProvidedModulesPlugin(configPath: string) {
  return {
    name: "turbo-gen-cli-provided",
    setup(build: {
      onResolve: (
        opts: { filter: RegExp },
        cb: (args: {
          path: string;
          resolveDir?: string;
          kind?: string;
        }) =>
          | { path: string; external?: boolean }
          | undefined
          | Promise<{ path: string; external?: boolean } | undefined>
      ) => void;
    }) {
      build.onResolve({ filter: /^[^./]/ }, (args) => {
        // Extract the package name from the specifier
        let packageName: string;
        if (args.path.startsWith("@")) {
          const parts = args.path.split("/");
          packageName = parts.slice(0, 2).join("/");
        } else {
          packageName = args.path.split("/")[0];
        }

        if (!CLI_PROVIDED_MODULES.includes(packageName)) {
          return undefined;
        }

        // Only intervene if esbuild's native resolution would fail.
        // Try to resolve from the user's project first.
        try {
          const userRequire = createRequire(
            args.resolveDir ? path.join(args.resolveDir, "_") : configPath
          );
          userRequire.resolve(args.path);
          // Found in user's project — let esbuild handle it normally.
          return undefined;
        } catch {
          // Not in user's project. Resolve from @turbo/gen's deps.
          try {
            const ownRequire = createRequire(__filename);
            const resolved = ownRequire.resolve(args.path);
            return { path: resolved };
          } catch {
            return undefined;
          }
        }
      });
    }
  };
}

function cleanupBundledConfigs() {
  for (const p of bundled) {
    try {
      fs.removeSync(p);
    } catch {
      // ignore cleanup failures
    }
  }
  bundled.clear();
}

function getWorkspaceGeneratorConfigs({ project }: { project: Project }) {
  const workspaceGeneratorConfigs: Array<{
    config: string;
    root: string;
  }> = [];
  for (const w of project.workspaceData.workspaces) {
    for (const configPath of SUPPORTED_WORKSPACE_GENERATOR_CONFIGS) {
      if (fs.existsSync(path.join(w.paths.root, configPath))) {
        workspaceGeneratorConfigs.push({
          config: path.join(w.paths.root, configPath),
          root: w.paths.root
        });
      }
    }
  }
  return workspaceGeneratorConfigs;
}

export async function runCustomGenerator({
  project,
  generator,
  bypassArgs,
  configPath
}: {
  project: Project;
  generator: string;
  bypassArgs?: Array<string>;
  configPath?: string;
}): Promise<void> {
  const plop = await getPlop({ project, configPath });
  if (!plop) {
    throw new GeneratorError("Unable to load generators", {
      type: "plop_unable_to_load_config"
    });
  }
  const gen = plop.getGenerator(generator) as PlopGenerator | undefined;

  if (!gen) {
    throw new GeneratorError(`Generator ${generator} not found`, {
      type: "plop_generator_not_found"
    });
  }

  const answers = (await gen.runPrompts(bypassArgs)) as Array<unknown>;
  const results = await gen.runActions(
    { ...answers, ...injectTurborepoData({ project, generator: gen }) },
    {
      onComment: (comment: string) => {
        logger.dimmed(comment);
      }
    }
  );

  if (results.failures.length > 0) {
    for (const f of results.failures) {
      if (f instanceof Error) {
        logger.error(`Error - ${f.message}`);
      } else {
        logger.error(`Error - ${f.error}. Unable to ${f.type} to "${f.path}"`);
      }
    }
    throw new GeneratorError(`Failed to run "${generator}" generator`, {
      type: "plop_error_running_generator"
    });
  }

  if (results.changes.length > 0) {
    logger.info("Changes made:");
    for (const c of results.changes) {
      if (c.path) {
        logger.item(`${c.path} (${c.type})`);
      }
    }
  }
}
