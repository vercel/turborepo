import path from "node:path";
import { builtinModules } from "node:module";
import fs from "fs-extra";
import type { Project } from "@turbo/workspaces";
import type { NodePlopAPI, PlopGenerator } from "node-plop";
import nodePlopModule from "node-plop";
import * as inquirerPrompts from "@inquirer/prompts";
import { searchUp, getTurboConfigs, logger } from "@turbo/utils";
import { GeneratorError } from "./error";

const { Separator } = inquirerPrompts;

// Bun's require() of CJS modules with Babel interop wraps exports differently
const nodePlop = (
  typeof nodePlopModule === "function"
    ? nodePlopModule
    : (nodePlopModule as { default: typeof nodePlopModule }).default
) as (
  plopfilePath: string,
  cfg?: { destBasePath?: string; force?: boolean }
) => NodePlopAPI | Promise<NodePlopAPI>;

const SUPPORTED_CONFIG_EXTENSIONS = ["ts", "js", "cjs"];
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
  // Bun handles TypeScript transpilation natively -- no tsx registration needed.

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
  gensWithDetails.forEach((g) => {
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
  });

  const gensWithSeparators: Array<Generator | InstanceType<typeof Separator>> =
    [];
  Object.keys(gensByWorkspace).forEach((group) => {
    gensWithSeparators.push(new Separator(group));
    gensWithSeparators.push(...gensByWorkspace[group]);
  });

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

// In standalone Bun-compiled binaries, node-plop's require() cannot resolve
// npm packages from dynamically loaded config files because the binary's module
// resolution doesn't see the project's node_modules. We use Bun.build() at
// runtime to bundle the user's config into a single CJS file before node-plop
// loads it.
//
// A subtlety: packages that the binary itself bundles (like @inquirer/prompts)
// won't exist on disk in the user's project. A Bun.build() plugin intercepts
// these unresolvable bare specifiers and redirects them to a globalThis
// registry where the binary's own module references are stored at runtime.
const bundled = new Set<string>();

// Modules bundled in the compiled binary that user configs may import.
// These are registered on globalThis before bundling so the generated CJS
// code can access them without require() (which can't resolve from the
// binary's virtual filesystem).
const BINARY_MODULES: Record<string, unknown> = {
  "@inquirer/prompts": inquirerPrompts
};

async function bundleConfigForLoading(configPath: string): Promise<string> {
  const BunAPI = (globalThis as unknown as { Bun?: { build: Function } }).Bun;
  if (!BunAPI?.build) return configPath;

  try {
    const outName = path
      .basename(configPath)
      .replace(/\.(ts|js|cjs)$/, ".turbo-gen-bundled.cjs");
    const outDir = path.dirname(configPath);
    const outPath = path.join(outDir, outName);

    // Expose binary-bundled modules on globalThis so the generated CJS code
    // can reference them without going through require().
    const g = globalThis as unknown as Record<string, unknown>;
    g.__turboGenModules = BINARY_MODULES;

    const result = await BunAPI.build({
      entrypoints: [configPath],
      outdir: outDir,
      naming: outName,
      target: "bun",
      format: "cjs",
      plugins: [binaryResolvePlugin(configPath)]
    });

    if (
      !result.success ||
      !result.outputs ||
      (result.outputs as Array<unknown>).length === 0
    ) {
      return configPath;
    }

    bundled.add(outPath);
    return outPath;
  } catch {
    return configPath;
  }
}

// Node builtins (with and without the "node:" prefix).
const NODE_BUILTINS = new Set<string>(builtinModules);
for (const m of builtinModules) {
  NODE_BUILTINS.add(`node:${m}`);
}

// Resolve a bare package specifier by walking up the directory tree.
// Inside a Bun compiled binary, createRequire().resolve() doesn't
// walk node_modules ancestors reliably, so we manually locate the
// package directory and read its package.json to find the entry point.
function resolvePackage(
  specifier: string,
  fromDir: string
): string | undefined {
  if (NODE_BUILTINS.has(specifier)) {
    return specifier;
  }

  // Extract the package name from the specifier (handles scoped packages
  // like @foo/bar and deep imports like foo/lib/thing).
  let packageName: string;
  let subpath: string | undefined;
  if (specifier.startsWith("@")) {
    const parts = specifier.split("/");
    packageName = parts.slice(0, 2).join("/");
    if (parts.length > 2) subpath = parts.slice(2).join("/");
  } else {
    const parts = specifier.split("/");
    packageName = parts[0];
    if (parts.length > 1) subpath = parts.slice(1).join("/");
  }

  // Walk up directories looking for node_modules/<packageName>
  let dir = fromDir;
  while (true) {
    const candidate = path.join(dir, "node_modules", packageName);
    if (fs.existsSync(candidate)) {
      if (subpath) {
        return path.join(candidate, subpath);
      }
      // Read the package's package.json to find the correct entry point.
      const pkgJsonPath = path.join(candidate, "package.json");
      if (fs.existsSync(pkgJsonPath)) {
        try {
          const pkg = fs.readJsonSync(pkgJsonPath) as Record<string, unknown>;
          const main = (pkg.main as string) || "index.js";
          return path.join(candidate, main);
        } catch {
          return path.join(candidate, "index.js");
        }
      }
      return path.join(candidate, "index.js");
    }
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }

  return undefined;
}

// Bun.build() plugin: when the user's config imports a bare specifier that
// can't be resolved from disk (the project's node_modules), check if it's a
// module the binary ships. If so, redirect to a virtual module that reads
// from the globalThis.__turboGenModules registry at runtime.
function binaryResolvePlugin(configPath: string) {
  return {
    name: "turbo-gen-binary-resolve",
    setup(build: {
      onResolve: (
        opts: { filter: RegExp },
        cb: (args: {
          path: string;
          resolveDir?: string;
        }) =>
          | { path: string; namespace?: string; external?: boolean }
          | undefined
      ) => void;
      onLoad: (
        opts: { filter: RegExp; namespace: string },
        cb: (args: { path: string }) => { contents: string; loader: string }
      ) => void;
    }) {
      build.onResolve({ filter: /^[^./]/ }, (args) => {
        const resolveDir = args.resolveDir || path.dirname(configPath);
        const resolved = resolvePackage(args.path, resolveDir);
        if (resolved) {
          if (path.isAbsolute(resolved)) {
            // Found on disk — give Bun the absolute path directly so it
            // doesn't need to resolve the bare specifier itself (which
            // fails inside the compiled binary).
            return { path: resolved };
          }
          // Node builtin (e.g. "fs", "node:path") — mark external so
          // the require() is preserved in the bundled output.
          return { path: resolved, external: true };
        }

        // Not on disk — redirect to virtual namespace if the binary has it
        if (args.path in BINARY_MODULES) {
          return { path: args.path, namespace: "turbo-gen-builtin" };
        }
        return undefined;
      });

      build.onLoad(
        { filter: /.*/, namespace: "turbo-gen-builtin" },
        (args) => ({
          contents: `module.exports = globalThis.__turboGenModules[${JSON.stringify(args.path)}];`,
          loader: "js"
        })
      );
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
  project.workspaceData.workspaces.forEach((w) => {
    for (const configPath of SUPPORTED_WORKSPACE_GENERATOR_CONFIGS) {
      if (fs.existsSync(path.join(w.paths.root, configPath))) {
        workspaceGeneratorConfigs.push({
          config: path.join(w.paths.root, configPath),
          root: w.paths.root
        });
      }
    }
  });
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
    results.failures.forEach((f) => {
      if (f instanceof Error) {
        logger.error(`Error - ${f.message}`);
      } else {
        logger.error(`Error - ${f.error}. Unable to ${f.type} to "${f.path}"`);
      }
    });
    throw new GeneratorError(`Failed to run "${generator}" generator`, {
      type: "plop_error_running_generator"
    });
  }

  if (results.changes.length > 0) {
    logger.info("Changes made:");
    results.changes.forEach((c) => {
      if (c.path) {
        logger.item(`${c.path} (${c.type})`);
      }
    });
  }
}
