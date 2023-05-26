import fs from "fs-extra";
import { Project } from "@turbo/workspaces";
import nodePlop, { NodePlopAPI, PlopGenerator } from "node-plop";
import { register } from "ts-node";
import path from "path";
import inquirer from "inquirer";
import { searchUp, getTurboConfigs, logger } from "@turbo/utils";
import { GeneratorError } from "./error";

const SUPPORTED_CONFIG_EXTENSIONS = ["ts", "js", "cjs"];
const TURBO_GENERATOR_DIRECTORY = path.join("turbo", "generators");

// config formats that will be automatically loaded from within workspaces
const SUPPORTED_WORKSPACE_GENERATOR_CONFIGS = SUPPORTED_CONFIG_EXTENSIONS.map(
  (ext) => path.join(TURBO_GENERATOR_DIRECTORY, `config.${ext}`)
);

// config formats that will be automatically loaded from the root (support plopfiles so that users with existing configurations can use them immediately)
const SUPPORTED_ROOT_GENERATOR_CONFIGS = [
  ...SUPPORTED_WORKSPACE_GENERATOR_CONFIGS,
  ...SUPPORTED_CONFIG_EXTENSIONS.map((ext) => path.join(`plopfile.${ext}`)),
];

export type Generator = PlopGenerator & {
  basePath: string;
  name: string;
};

// init ts-node for plop to support ts configs
register({
  transpileOnly: true,
});

export function getPlop({
  project,
  configPath,
}: {
  project: Project;
  configPath?: string;
}): NodePlopAPI | undefined {
  // fetch all the workspace generator configs
  const workspaceConfigs = getWorkspaceGeneratorConfigs({ project });
  let plop: NodePlopAPI | undefined = undefined;

  if (configPath) {
    try {
      plop = nodePlop(configPath, {
        destBasePath: configPath,
        force: false,
      });
    } catch (e) {
      // skip
    }
  } else {
    // look for a root config
    for (const configPath of SUPPORTED_ROOT_GENERATOR_CONFIGS) {
      const plopFile = path.join(project.paths.root, configPath);
      try {
        plop = nodePlop(plopFile, {
          destBasePath: project.paths.root,
          force: false,
        });
        break;
      } catch (e) {
        // skip
      }
    }

    if (!plop && workspaceConfigs.length > 0) {
      // if no root config, use the first workspace config as the entrypoint
      plop = nodePlop(workspaceConfigs[0].config, {
        destBasePath: workspaceConfigs[0].root,
        force: false,
      });
      workspaceConfigs.shift();
    }
  }

  if (plop) {
    // add in all the workspace configs
    workspaceConfigs.forEach((c) => {
      try {
        plop?.load(c.config, {
          destBasePath: c.root,
          force: false,
        });
      } catch (e) {
        console.error(e);
      }
    });
  }

  return plop;
}

export function getCustomGenerators({
  project,
  configPath,
}: {
  project: Project;
  configPath?: string;
}): Array<Generator | inquirer.Separator> {
  const plop = getPlop({ project, configPath });

  if (!plop) {
    return [];
  }

  const gens = plop.getGeneratorList();
  const gensWithDetails = gens.map((g) => plop.getGenerator(g.name));

  // group by workspace
  const gensByWorkspace: Record<string, Array<Generator>> = {};
  gensWithDetails.forEach((g) => {
    const generatorDetails = g as Generator;
    const gensWorkspace = project.workspaceData.workspaces.find((w) => {
      if (generatorDetails.basePath === project.paths.root) {
        return false;
      }
      // we can strip two directories to get the workspace root
      const parts = generatorDetails.basePath.split(path.sep);
      // generators
      parts.pop();
      // turbo
      parts.pop();
      const workspaceRoot = path.join("/", ...parts);
      return workspaceRoot == w.paths.root;
    });

    if (gensWorkspace) {
      if (!gensByWorkspace[gensWorkspace.name]) {
        gensByWorkspace[gensWorkspace.name] = [];
      }
      gensByWorkspace[gensWorkspace.name].push(generatorDetails);
    } else {
      if (!gensByWorkspace["root"]) {
        gensByWorkspace["root"] = [];
      }
      gensByWorkspace["root"].push(generatorDetails);
    }
  });

  // add in separators to group by workspace
  const gensWithSeparators: Array<Generator | inquirer.Separator> = [];
  const lastGroup = undefined;
  Object.keys(gensByWorkspace).forEach((group) => {
    if (group !== lastGroup) {
      gensWithSeparators.push(new inquirer.Separator(group));
    }
    gensWithSeparators.push(...gensByWorkspace[group]);
  });

  return gensWithSeparators;
}

export function getCustomGenerator({
  project,
  generator,
  configPath,
}: {
  project: Project;
  generator: string;
  configPath?: string;
}): string | undefined {
  const plop = getPlop({ project, configPath });
  if (!plop) {
    return undefined;
  }

  try {
    const gen = plop.getGenerator(generator);
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
  generator,
}: {
  project: Project;
  generator: PlopGenerator & { basePath?: string };
}) {
  const paths = {
    root: project.paths.root,
    workspace: generator.basePath
      ? searchUp({ cwd: generator.basePath, target: "package.json" })
      : undefined,
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
      configs: turboConfigs,
    },
  };
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
          root: w.paths.root,
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
  configPath,
}: {
  project: Project;
  generator: string;
  bypassArgs?: Array<string>;
  configPath?: string;
}): Promise<void> {
  const plop = getPlop({ project, configPath });
  if (!plop) {
    throw new GeneratorError("Unable to load generators", {
      type: "plop_unable_to_load_config",
    });
  }
  const gen: PlopGenerator & { basePath?: string } =
    plop.getGenerator(generator);

  if (!gen) {
    throw new GeneratorError(`Generator ${generator} not found`, {
      type: "plop_generator_not_found",
    });
  }

  const answers = await gen.runPrompts(bypassArgs);
  const results = await gen.runActions(
    { ...answers, ...injectTurborepoData({ project, generator: gen }) },
    {
      onComment: (comment: string) => {
        console.info(comment);
      },
    }
  );

  if (results.failures && results.failures.length > 0) {
    // log all errors:
    results.failures.forEach((f) => {
      if (f instanceof Error) {
        logger.error(`Error - ${f.message}`);
      } else {
        logger.error(`Error - ${f.error}. Unable to ${f.type} to "${f.path}"`);
      }
    });
    throw new GeneratorError(`Failed to run "${generator}" generator`, {
      type: "plop_error_running_generator",
    });
  }

  if (results.changes && results.changes.length > 0) {
    logger.info("Changes made:");
    results.changes.forEach((c) => {
      if (c.path) {
        logger.item(`${c.path} (${c.type})`);
      }
    });
  }
}
