import path from "node:path";
import fs from "node:fs";
import crypto from "node:crypto";
import type { Rule } from "eslint";
import type { Node, MemberExpression } from "estree";
import {
  type PackageJson,
  logger,
  searchUp,
  clearConfigCaches,
} from "@turbo/utils";
import { frameworks } from "@turbo/types";
import { RULES } from "../constants";
import { Project, getWorkspaceFromFilePath } from "../utils/calculate-inputs";

const debug = process.env.RUNNER_DEBUG
  ? logger.info
  : (_: string) => {
      /* noop */
    };

// Module-level caches to share state across all files in a single ESLint run
interface CachedProject {
  project: Project;
  turboConfigHashes: Map<string, string>;
  configPaths: Array<string>;
}

const projectCache = new Map<string, CachedProject>();
const frameworkEnvCache = new Map<string, Set<RegExp>>();
const packageJsonDepCache = new Map<string, Set<string>>();

export interface RuleContextWithOptions extends Rule.RuleContext {
  options: Array<{
    cwd?: string;
    allowList?: Array<string>;
  }>;
}

const meta: Rule.RuleMetaData = {
  type: "problem",
  docs: {
    description:
      "Do not allow the use of `process.env` without including the env key in any turbo.json",
    category: "Configuration Issues",
    recommended: true,
    url: `https://github.com/vercel/turborepo/tree/main/packages/eslint-plugin-turbo/docs/rules/${RULES.noUndeclaredEnvVars}.md`,
  },
  schema: [
    {
      type: "object",
      default: {},
      additionalProperties: false,
      properties: {
        // override cwd, primarily exposed for easier testing
        cwd: {
          require: false,
          type: "string",
        },
        allowList: {
          default: [],
          type: "array",
          items: {
            type: "string",
          },
        },
      },
    },
  ],
};

/**
 * Normalize the value of the cwd
 * Extracted from eslint
 * SPDX-License-Identifier: MIT
 */
function normalizeCwd(
  cwd: string | undefined,
  options: RuleContextWithOptions["options"]
): string | undefined {
  if (options[0]?.cwd) {
    return options[0].cwd;
  }

  if (cwd) {
    return cwd;
  }
  if (typeof process === "object") {
    return process.cwd();
  }

  return undefined;
}

/** for a given `package.json` file path, this will compile a Set of that package's listed dependencies */
const packageJsonDependencies = (filePath: string): Set<string> => {
  const cached = packageJsonDepCache.get(filePath);
  if (cached) {
    return cached;
  }

  // get the contents of the package.json
  let packageJsonString;

  try {
    packageJsonString = fs.readFileSync(filePath, "utf-8");
  } catch (e) {
    logger.error(`Could not read package.json at ${filePath}`);
    const emptySet = new Set<string>();
    packageJsonDepCache.set(filePath, emptySet);
    return emptySet;
  }

  let packageJson: PackageJson;
  try {
    packageJson = JSON.parse(packageJsonString) as PackageJson;
  } catch (e) {
    logger.error(`Could not parse package.json at ${filePath}`);
    const emptySet = new Set<string>();
    packageJsonDepCache.set(filePath, emptySet);
    return emptySet;
  }

  const dependencies = (
    [
      "dependencies",
      "devDependencies",
      "peerDependencies",
      // intentionally not including `optionalDependencies` or `bundleDependencies` because at the time of writing they are not used for any of the frameworks we support
    ] as const
  )
    .flatMap((key) => Object.keys(packageJson[key] ?? {}))
    .reduce((acc, dependency) => acc.add(dependency), new Set<string>());

  packageJsonDepCache.set(filePath, dependencies);
  return dependencies;
};

/**
 * Find turbo.json or turbo.jsonc in a directory if it exists
 */
function findTurboConfigInDir(dirPath: string): string | null {
  const turboJsonPath = path.join(dirPath, "turbo.json");
  const turboJsoncPath = path.join(dirPath, "turbo.jsonc");

  if (fs.existsSync(turboJsonPath)) {
    return turboJsonPath;
  }
  if (fs.existsSync(turboJsoncPath)) {
    return turboJsoncPath;
  }
  return null;
}

/**
 * Get all turbo config file paths that are currently loaded in the project
 */
function getTurboConfigPaths(project: Project): Array<string> {
  const paths: Array<string> = [];

  // Add root turbo config if it exists and is loaded
  if (project.projectRoot?.turboConfig) {
    const configPath = findTurboConfigInDir(project.projectRoot.workspacePath);
    if (configPath) {
      paths.push(configPath);
    }
  }

  // Add workspace turbo configs that are loaded
  for (const workspace of project.projectWorkspaces) {
    if (workspace.turboConfig) {
      const configPath = findTurboConfigInDir(workspace.workspacePath);
      if (configPath) {
        paths.push(configPath);
      }
    }
  }

  return paths;
}

/**
 * Scan filesystem for all turbo.json/turbo.jsonc files across all workspaces.
 * This scans ALL workspaces regardless of whether they currently have turboConfig loaded,
 * allowing detection of newly created turbo.json files.
 */
function scanForTurboConfigs(project: Project): Array<string> {
  const paths: Array<string> = [];

  // Check root turbo config
  if (project.projectRoot) {
    const configPath = findTurboConfigInDir(project.projectRoot.workspacePath);
    if (configPath) {
      paths.push(configPath);
    }
  }

  // Check ALL workspaces for turbo configs (not just those with turboConfig already loaded)
  for (const workspace of project.projectWorkspaces) {
    const configPath = findTurboConfigInDir(workspace.workspacePath);
    if (configPath) {
      paths.push(configPath);
    }
  }

  return paths;
}

/**
 * Compute hashes for all turbo.config(c) files
 */
function computeTurboConfigHashes(
  configPaths: Array<string>
): Map<string, string> {
  const hashes = new Map<string, string>();

  for (const configPath of configPaths) {
    const content = fs.readFileSync(configPath, "utf-8");
    const hash = crypto.createHash("md5").update(content).digest("hex");
    hashes.set(configPath, hash);
  }

  return hashes;
}

/**
 * Check if a single config file has changed by comparing its hash
 */
function hasConfigChanged(filePath: string, expectedHash: string): boolean {
  try {
    const content = fs.readFileSync(filePath, "utf-8");
    const currentHash = crypto.createHash("md5").update(content).digest("hex");
    return currentHash !== expectedHash;
  } catch {
    // File no longer exists or is unreadable
    return true;
  }
}

/**
 * Turborepo does some nice framework detection based on the dependencies in the package.json.  This function ports that logic to this ESLint rule.
 *
 * Imagine you have a Vue app.  That means you have Vue in your `package.json` dependencies.  This function will return a list of regular expressions that match the environment variables that Vue depends on, which is information encoded into the `frameworks.json` file.  In Vue's case, it would return the regex `VUE_APP_*` since you have `@vue/cli-service` in your dependencies.
 */
const frameworkEnvMatches = (filePath: string): Set<RegExp> => {
  const directory = path.dirname(filePath);
  const packageJsonDir = searchUp({ cwd: directory, target: "package.json" });
  if (!packageJsonDir) {
    logger.error(`Could not determine package for ${filePath}`);
    return new Set<RegExp>();
  }

  // Use package.json path as cache key since all files in same package share the same framework config
  const cacheKey = `${packageJsonDir}/package.json`;
  const cached = frameworkEnvCache.get(cacheKey);
  if (cached) {
    return cached;
  }

  debug(`found package.json in: ${packageJsonDir}`);

  const dependencies = packageJsonDependencies(cacheKey);
  const hasDependency = (dep: string) => dependencies.has(dep);
  debug(`dependencies for ${filePath}: ${Array.from(dependencies).join(",")}`);

  const result = frameworks.reduce(
    (
      acc,
      {
        dependencyMatch: { dependencies: searchDependencies, strategy },
        envWildcards,
      }
    ) => {
      const hasMatch =
        strategy === "all"
          ? searchDependencies.every(hasDependency)
          : searchDependencies.some(hasDependency);

      if (hasMatch) {
        return new Set([
          ...acc,
          ...envWildcards.map((envWildcard) => RegExp(envWildcard)),
        ]);
      }
      return acc;
    },
    new Set<RegExp>()
  );

  frameworkEnvCache.set(cacheKey, result);
  return result;
};

function create(context: RuleContextWithOptions): Rule.RuleListener {
  const { options } = context;

  const allowList: Array<string> = options[0]?.allowList || [];
  let regexAllowList: Array<RegExp> = [];
  allowList.forEach((allowed) => {
    try {
      regexAllowList.push(new RegExp(allowed));
    } catch (err) {
      // log the error, but just move on without this allowList entry
      logger.error(`Unable to convert "${allowed}" to regex`);
    }
  });

  const filename = context.filename;
  debug(`Checking file: ${filename}`);

  const matches = frameworkEnvMatches(filename);
  regexAllowList = [...regexAllowList, ...matches];
  debug(
    `Allow list: ${regexAllowList.map((r) => r.source).join(",")}, ${
      regexAllowList.length
    }`
  );

  const cwd = normalizeCwd(context.cwd ? context.cwd : undefined, options);

  // Use cached Project instance to avoid expensive re-initialization for every file
  const projectKey = cwd ?? process.cwd();
  const cachedProject = projectCache.get(projectKey);
  let project: Project;

  if (!cachedProject) {
    project = new Project(cwd);
    if (project.valid()) {
      const configPaths = getTurboConfigPaths(project);
      const hashes = computeTurboConfigHashes(configPaths);
      projectCache.set(projectKey, {
        project,
        turboConfigHashes: hashes,
        configPaths,
      });
      debug(`Cached new project for ${projectKey}`);
    }
  } else {
    project = cachedProject.project;

    // Check if any turbo.json(c) configs have changed
    try {
      const currentConfigPaths = scanForTurboConfigs(project);

      // Quick path comparison - cheapest check first
      const pathsUnchanged =
        currentConfigPaths.length === cachedProject.configPaths.length &&
        currentConfigPaths.every((p, i) => p === cachedProject.configPaths[i]);

      if (!pathsUnchanged) {
        // Paths changed (added/removed configs), must reload
        debug(`Turbo config paths changed for ${projectKey}, reloading...`);
        const newHashes = computeTurboConfigHashes(currentConfigPaths);
        project.reload();
        cachedProject.turboConfigHashes = newHashes;
        cachedProject.configPaths = currentConfigPaths;
      } else {
        // Paths unchanged - check if any file content changed (early exit on first change)
        let contentChanged = false;
        for (const [
          filePath,
          expectedHash,
        ] of cachedProject.turboConfigHashes) {
          if (hasConfigChanged(filePath, expectedHash)) {
            contentChanged = true;
            break;
          }
        }

        if (contentChanged) {
          debug(`Turbo config content changed for ${projectKey}, reloading...`);
          const newHashes = computeTurboConfigHashes(currentConfigPaths);
          project.reload();
          cachedProject.turboConfigHashes = newHashes;
          cachedProject.configPaths = currentConfigPaths;
        }
      }
    } catch (error) {
      // Config file was deleted or is unreadable, reload project
      debug(`Error computing hashes for ${projectKey}, reloading...`);
      project.reload();
      const configPaths = scanForTurboConfigs(project);
      cachedProject.turboConfigHashes = computeTurboConfigHashes(configPaths);
      cachedProject.configPaths = configPaths;
    }
  }

  if (!project.valid()) {
    return {};
  }

  const filePath = context.physicalFilename;
  const hasWorkspaceConfigs = project.projectWorkspaces.some(
    (workspaceConfig) => Boolean(workspaceConfig.turboConfig)
  );
  const workspaceConfig = getWorkspaceFromFilePath(
    project.projectWorkspaces,
    filePath
  );

  const checkKey = (node: Node, envKey?: string) => {
    if (!envKey) {
      return {};
    }

    if (regexAllowList.some((regex) => regex.test(envKey))) {
      return {};
    }

    const configured = project.test(workspaceConfig?.workspaceName, envKey);

    if (configured) {
      return {};
    }
    let message = `{{ envKey }} is not listed as a dependency in ${
      hasWorkspaceConfigs ? "root turbo.json" : "turbo.json"
    }`;
    if (workspaceConfig?.turboConfig) {
      if (cwd) {
        // if we have a cwd, we can provide a relative path to the workspace config
        message = `{{ envKey }} is not listed as a dependency in the root turbo.json or workspace (${path.relative(
          cwd,
          workspaceConfig.workspacePath
        )}) turbo.json`;
      } else {
        message = `{{ envKey }} is not listed as a dependency in the root turbo.json or workspace turbo.json`;
      }
    }

    context.report({
      node,
      message,
      data: { envKey },
    });
  };

  const isComputed = (
    node: MemberExpression & Rule.NodeParentExtension
  ): boolean => {
    if ("computed" in node.parent) {
      return node.parent.computed;
    }

    return false;
  };

  const isProcessEnv = (node: MemberExpression): boolean => {
    return (
      "name" in node.object &&
      "name" in node.property &&
      node.object.name === "process" &&
      node.property.name === "env"
    );
  };

  const isImportMetaEnv = (node: MemberExpression): boolean => {
    return (
      node.object.type === "MetaProperty" &&
      node.object.meta.name === "import" &&
      node.object.property.name === "meta" &&
      node.property.type === "Identifier" &&
      node.property.name === "env"
    );
  };

  return {
    MemberExpression(node) {
      // we only care about complete process env declarations and non-computed keys
      if (isProcessEnv(node) || isImportMetaEnv(node)) {
        // we're doing something with process.env
        if (!isComputed(node)) {
          // destructuring from process.env
          if ("id" in node.parent && node.parent.id?.type === "ObjectPattern") {
            const values = node.parent.id.properties.values();
            Array.from(values).forEach((item) => {
              if ("key" in item && "name" in item.key) {
                checkKey(node.parent, item.key.name);
              }
            });
          }

          // accessing key on process.env
          else if (
            "property" in node.parent &&
            "name" in node.parent.property
          ) {
            checkKey(node.parent, node.parent.property.name);
          }
        } else if (
          "property" in node.parent &&
          node.parent.property.type === "Literal" &&
          typeof node.parent.property.value === "string"
        ) {
          // If we're indexing by a literal, we can check it
          checkKey(node.parent, node.parent.property.value);
        }
      }
    },
  };
}

/**
 * Clear all module-level caches. This is primarily useful for test isolation.
 */
export function clearCache(): void {
  projectCache.clear();
  frameworkEnvCache.clear();
  packageJsonDepCache.clear();
  clearConfigCaches();
}

const rule = { create, meta };
export default rule;
