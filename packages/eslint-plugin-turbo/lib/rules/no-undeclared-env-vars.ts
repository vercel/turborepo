import path from "node:path";
import { existsSync, readFileSync } from "node:fs";
import type { Rule } from "eslint";
import type { Node, MemberExpression } from "estree";
import { logger } from "@turbo/utils";
import { RULES } from "../constants";
import { Project, getWorkspaceFromFilePath } from "../utils/calculate-inputs";

/** set this to true if debugging this rule */
const debugging = "".length === 0;
const debug = debugging
  ? logger.info
  : (_: string) => {
      /* noop */
    };

interface Framework {
  slug: string;
  envWildcards: Array<string>;
  dependencyMatch: {
    strategy: "all" | "some";
    dependencies: Array<string>;
  };
}

export interface RuleContextWithOptions extends Rule.RuleContext {
  options: Array<{
    cwd?: string;
    allowList?: Array<string>;
  }>;
}

/** recursively find the closest package.json from the given directory */
const findClosestPackageJson = (currentDir: string): string | null => {
  debug(`searching for package.json in ${currentDir}`);
  const packageJsonPath = path.join(currentDir, "package.json");

  // Check if package.json exists in the current directory
  if (existsSync(packageJsonPath)) {
    return packageJsonPath;
  }

  // Get the parent directory
  const parentDir = path.dirname(currentDir);

  // If we've reached the root directory, stop searching
  if (parentDir === currentDir) {
    return null;
  }

  // Recursively search in the parent directory
  return findClosestPackageJson(parentDir);
};

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

const packageJsonDependencies = (filePath: string): Set<string> => {
  // get the contents of the package.json
  const packageJsonString = readFileSync(filePath, "utf-8");
  const packageJson = JSON.parse(packageJsonString) as Record<
    string,
    Record<string, string>
  >;
  return ["dependencies", "devDependencies", "peerDependencies"].reduce(
    (acc, key) => {
      if (key in packageJson) {
        Object.keys(packageJson[key]).forEach((dependency) => {
          acc.add(dependency);
        });
      }
      return acc;
    },
    new Set<string>()
  );
};

const frameworksJsonString = readFileSync(
  "../../../../crates/turborepo-lib/src/frameworks.json",
  "utf-8"
);
const frameworks = JSON.parse(frameworksJsonString) as Array<Framework>;

/**
 * Turborepo does some nice framework detection based on the dependencies in the package.json.  This function ports that logic to the eslint rule.
 *
 * So, essentially, imagine you have a Vue app.  That means you have Vue in your dependencies.  This function will return a list of regular expressions that match the environment variables that Vue uses, which is information encoded into the `frameworks.json` file.  In Vue's case, it would return a regex that matches `VUE_APP_*` if you have `@vue/cli-service` in your dependencies.
 */
const matchesFramework = (filePath: string): Array<RegExp> => {
  const directory = path.dirname(filePath);
  const packageJsonPath = findClosestPackageJson(directory);
  if (!packageJsonPath) {
    logger.error(`No package.json found connected to ${filePath}`);
    return [];
  }
  debug(`found package.json: ${packageJsonPath}`);

  const dependencies = packageJsonDependencies(packageJsonPath);
  debug(`dependencies for ${filePath}: ${Array.from(dependencies).join(",")}`);
  const matches = frameworks.reduce((acc, framework) => {
    const dependenciesMatch = framework.dependencyMatch.dependencies.some(
      (dependency) => dependencies.has(dependency)
    );
    if (dependenciesMatch) {
      framework.envWildcards.forEach((wildcard) =>
        acc.add(new RegExp(wildcard))
      );
    }
    return acc;
  }, new Set<RegExp>());
  return Array.from(matches);
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

  const filename = context.getFilename();
  debug(`Checking file: ${filename}`);

  const matches = matchesFramework(filename);
  regexAllowList = [...regexAllowList, ...matches];
  debug(
    `Allow list: ${regexAllowList.map((r) => r.source).join(",")}, ${
      regexAllowList.length
    }`
  );

  const cwd = normalizeCwd(
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- needed to support older eslint versions
    context.getCwd ? context.getCwd() : undefined,
    options
  );

  const project = new Project(cwd);
  if (!project.valid()) {
    return {};
  }

  const filePath = context.getPhysicalFilename();
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
    let message = `1 {{ envKey }} is not listed as a dependency in ${
      hasWorkspaceConfigs ? "root turbo.json" : "turbo.json"
    }`;
    if (workspaceConfig?.turboConfig) {
      if (cwd) {
        // if we have a cwd, we can provide a relative path to the workspace config
        message = `2 {{ envKey }} is not listed as a dependency in the root turbo.json or workspace (${path.relative(
          cwd,
          workspaceConfig.workspacePath
        )}) turbo.json`;
      } else {
        message = `3 {{ envKey }} is not listed as a dependency in the root turbo.json or workspace turbo.json`;
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

const rule = { create, meta };
export default rule;
