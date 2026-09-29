import path from "node:path";
import fs from "node:fs";
import type { Rule } from "eslint";
import type { Node, MemberExpression } from "estree";
import {
  type PackageJson,
  logger,
  searchUp,
  clearConfigCaches
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
interface TurboConfigStat {
  mtimeMs: number;
  size: number;
}

interface CachedProject {
  project: Project;
  turboConfigStats: Map<string, TurboConfigStat>;
  lastValidatedAt: number;
}

const projectCache = new Map<string, CachedProject>();
const frameworkEnvCache = new Map<string, Set<RegExp>>();
const packageJsonDepCache = new Map<string, Set<string>>();

// ESLint creates this rule once per source file. Without coalescing, every
// creation re-scans all workspaces for turbo.json/turbo.jsonc, which turns
// config validation into O(source files x workspace configs) filesystem work
// even when nothing changed. Rate-limiting validation to one sweep per
// interval amortizes it across a batch of files (a CLI lint run or an editor
// lint pass), while subsequent lint runs still pick up config changes.
const CONFIG_VALIDATION_INTERVAL_MS = 1_000;

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
    recommended: true,
    url: `https://github.com/vercel/turborepo/tree/main/packages/eslint-plugin-turbo/docs/rules/${RULES.noUndeclaredEnvVars}.md`
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
          type: "string"
        },
        allowList: {
          default: [],
          type: "array",
          items: {
            type: "string"
          }
        }
      }
    }
  ]
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
      "peerDependencies"
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
 * Get a cheap signature (mtime + size) for a turbo config file, or `null` if
 * the file is missing or unreadable. Unlike content hashing, this never reads
 * the file's contents.
 */
function getTurboConfigStat(filePath: string): TurboConfigStat | null {
  try {
    const stats = fs.statSync(filePath);
    return { mtimeMs: stats.mtimeMs, size: stats.size };
  } catch {
    // File no longer exists or is unreadable
    return null;
  }
}

/**
 * Compute stat signatures for all turbo.config(c) files, skipping any that
 * cannot be stat'ed
 */
function computeTurboConfigStats(
  configPaths: Array<string>
): Map<string, TurboConfigStat> {
  const stats = new Map<string, TurboConfigStat>();

  for (const configPath of configPaths) {
    const stat = getTurboConfigStat(configPath);
    if (stat) {
      stats.set(configPath, stat);
    }
  }

  return stats;
}

/**
 * Reload the cached project and refresh the tracked turbo config stats
 */
function reloadCachedProject(cachedProject: CachedProject): void {
  cachedProject.project.reload();
  cachedProject.turboConfigStats = computeTurboConfigStats(
    scanForTurboConfigs(cachedProject.project)
  );
}

/**
 * Check whether the cached project still matches the turbo configs on disk,
 * reloading the project if any config was added, removed, or modified.
 * Configs are compared by stat metadata (mtime + size) rather than content
 * hashes so unchanged configs are never re-read.
 */
function validateCachedProject(cachedProject: CachedProject): void {
  const currentStats = computeTurboConfigStats(
    scanForTurboConfigs(cachedProject.project)
  );
  const previousStats = cachedProject.turboConfigStats;

  const statsUnchanged =
    currentStats.size === previousStats.size &&
    [...currentStats].every(([configPath, stat]) => {
      const previousStat = previousStats.get(configPath);
      return (
        previousStat !== undefined &&
        previousStat.mtimeMs === stat.mtimeMs &&
        previousStat.size === stat.size
      );
    });

  if (!statsUnchanged) {
    reloadCachedProject(cachedProject);
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
        envWildcards
      }
    ) => {
      const hasMatch =
        strategy === "all"
          ? searchDependencies.every(hasDependency)
          : searchDependencies.some(hasDependency);

      if (hasMatch) {
        return new Set([
          ...acc,
          ...envWildcards.map((envWildcard) => RegExp(envWildcard))
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
  for (const allowed of allowList) {
    try {
      regexAllowList.push(new RegExp(allowed));
    } catch (err) {
      // log the error, but just move on without this allowList entry
      logger.error(`Unable to convert "${allowed}" to regex`);
    }
  }

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
      projectCache.set(projectKey, {
        project,
        turboConfigStats: computeTurboConfigStats(scanForTurboConfigs(project)),
        lastValidatedAt: Date.now()
      });
      debug(`Cached new project for ${projectKey}`);
    }
  } else {
    project = cachedProject.project;

    // ESLint invokes this rule once per source file, so only validate the
    // cached project's turbo configs at most once per interval. Batches of
    // unchanged files skip the filesystem entirely, while edits, added or
    // removed configs, and subsequent lint runs (e.g. from a persistent
    // editor) still trigger a reload.
    const now = Date.now();
    if (now - cachedProject.lastValidatedAt >= CONFIG_VALIDATION_INTERVAL_MS) {
      cachedProject.lastValidatedAt = now;
      try {
        validateCachedProject(cachedProject);
      } catch (error) {
        // Config file was deleted or is unreadable, reload project
        debug(`Error validating configs for ${projectKey}, reloading...`);
        reloadCachedProject(cachedProject);
      }
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
      data: { envKey }
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
            for (const item of Array.from(values)) {
              if ("key" in item && "name" in item.key) {
                checkKey(node.parent, item.key.name);
              }
            }
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
    }
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
