import type { Rule } from "eslint";
import fs from "fs";
import path from "path";
import { Node, MemberExpression } from "estree";
import { RULES } from "../constants";
import getEnvVarDependencies from "../utils/getEnvVarDependencies";
import { getTurboConfigs } from "@turbo/utils";
import { wildcardTests } from "../utils/wildcard-processing";
import { dotEnv } from "../utils/dotenv-processing";

const meta: Rule.RuleMetaData = {
  type: "problem",
  docs: {
    description:
      "Do not allow the use of `process.env` without including the env key in any turbo.json",
    category: "Configuration Issues",
    recommended: true,
    url: `https://github.com/vercel/turbo/tree/main/packages/eslint-plugin-turbo/docs/rules/${RULES.noUndeclaredEnvVars}.md`,
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
  options: Array<any>
): string | undefined {
  if (options?.[0]?.cwd) {
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

type EnvVar = string;
type EnvTest = (variable: EnvVar) => boolean;
interface EnvContext {
  legacyConfig: EnvTest;
  env: EnvTest;
  passThroughEnv: EnvTest;
  dotEnv: EnvTest;
}

interface EnvConfig {
  legacyConfig: string[] | undefined;
  env: string[] | undefined;
  passThroughEnv: string[] | null | undefined;
  dotEnv: string[] | null | undefined;
}

type TestContext = {
  allowList: EnvTest[];
  global: EnvContext;
  globalTasks: {
    [script: string]: EnvContext;
  };
  workspaceTasks: {
    [workspace: string]: {
      [script: string]: EnvContext;
    };
  };
};

function envContextTestArray(envContext: EnvContext) {
  return [
    envContext.legacyConfig,
    envContext.env,
    envContext.passThroughEnv,
    envContext.dotEnv,
  ];
}

function workspaceNameFromFilePath(filePath: string): string | null {
  let workspacePaths = Object.keys(workspaceLookup);
  let possibleWorkspacePaths = workspacePaths
    .filter((workspacePath) => filePath.startsWith(workspacePath))
    .sort();

  if (possibleWorkspacePaths.length > 0) {
    return workspaceLookup[possibleWorkspacePaths[0]];
  }

  return null;
}

function checkForInclusion(
  testContext: TestContext,
  workspaceName: string | null,
  variable: EnvVar
): boolean {
  const tests = [
    testContext.allowList,
    envContextTestArray(testContext.global),
    ...Object.values(testContext.globalTasks).map((context) =>
      envContextTestArray(context)
    ),
  ];

  if (workspaceName !== null) {
    tests.push(
      ...Object.values(testContext.workspaceTasks[workspaceName]).map(
        (context) => envContextTestArray(context)
      )
    );
  }

  return tests.flat().findIndex((test) => test(variable)) === -1;
}

function TestFalse(_: EnvVar) {
  return false;
}
function getEnvContext(cwd: string, config: EnvConfig) {
  const envContext: EnvContext = {
    legacyConfig: TestFalse,
    env: TestFalse,
    passThroughEnv: TestFalse,
    dotEnv: TestFalse,
  };

  // a. Check for legacy configuration
  if (config.legacyConfig) {
    const legacyConfigEnvVars = config.legacyConfig
      // filter for env vars
      .filter((dep) => dep.startsWith("$"))
      // remove leading $
      .map((variable) => variable.slice(1));

    // Conditionally add this function.
    if (legacyConfigEnvVars.length) {
      const dependsOnEnvSet = new Set(legacyConfigEnvVars);
      envContext.legacyConfig = (variable: EnvVar) =>
        dependsOnEnvSet.has(variable);
    }
  }

  // b. Check the env configuration.
  if (config.env && config.env.length > 0) {
    const testRegexes = wildcardTests(config.env);
    envContext.env = (variable: EnvVar) => {
      return (
        testRegexes.inclusions.test(variable) &&
        !testRegexes.exclusions.test(variable)
      );
    };
  }

  // c. Check the passThroughEnv configuration.
  if (config.passThroughEnv && config.passThroughEnv.length > 0) {
    const testRegexes = wildcardTests(config.passThroughEnv);
    envContext.passThroughEnv = (variable: EnvVar) => {
      return (
        testRegexes.inclusions.test(variable) &&
        !testRegexes.exclusions.test(variable)
      );
    };
  }

  // d. Check to see if the variable is accounted for by dotEnv.
  if (config.dotEnv && config.dotEnv.length > 0) {
    const dotEnvEnvSet = dotEnv(cwd, config.dotEnv);
    envContext.dotEnv = (variable: EnvVar) => dotEnvEnvSet.has(variable);
  }

  return envContext;
}

const workspaceLookup: { [path: string]: string } = {};
function getWorkspaceName(workspacePath: string): string {
  if (workspaceLookup[workspacePath]) {
    return workspaceLookup[workspacePath];
  }

  const packageJsonContents = fs.readFileSync(
    path.join(workspacePath, "package.json"),
    "utf8"
  );
  const packageJson = JSON.parse(packageJsonContents);

  if (packageJson.name) {
    workspaceLookup[workspacePath] = packageJson.name;
    return packageJson.name;
  }

  throw new Error(`Unable to discover workspace name: ${workspacePath}`);
}

function create(context: Rule.RuleContext): Rule.RuleListener {
  const { options, getPhysicalFilename } = context;

  // This will get bundled up as a neat little object later.
  let allowListTests: EnvTest[];
  let globalEnvContext: EnvContext = {
    legacyConfig: TestFalse,
    env: TestFalse,
    passThroughEnv: TestFalse,
    dotEnv: TestFalse,
  };
  let globalTasks: {
    [script: string]: EnvContext;
  } = {};
  let workspaceTasks: {
    [workspace: string]: {
      [script: string]: EnvContext;
    };
  } = {};

  // 1. Create the tests for the regex allowList.
  const allowList: Array<string> = options?.[0]?.allowList || [];
  const regexAllowList: Array<RegExp> = [];
  allowList.forEach((allowed) => {
    try {
      regexAllowList.push(new RegExp(allowed));
    } catch (err) {
      // log the error, but just move on without this allowList entry
      console.error(`Unable to convert "${allowed}" to regex`);
    }
  });

  allowListTests = regexAllowList.map((allowedRegex) => {
    return (variable: EnvVar) => {
      return allowedRegex.test(variable);
    };
  });

  const cwd = normalizeCwd(
    context.getCwd ? context.getCwd() : undefined,
    options
  );

  // Process the project.
  const turboJsons = getTurboConfigs(cwd);

  // 2. Create the tests for the root turbo.json.
  const rootTurboJson = turboJsons.find((turboJson) => turboJson.isRootConfig);
  if (rootTurboJson && !("extends" in rootTurboJson.config)) {
    // second clause is a type assertion
    // First, there is a default, unconditional, global context.
    globalEnvContext = getEnvContext(rootTurboJson.turboConfigPath, {
      legacyConfig: rootTurboJson.config.globalDependencies,
      env: rootTurboJson.config.globalEnv,
      passThroughEnv: rootTurboJson.config.globalPassThroughEnv,
      dotEnv: rootTurboJson.config.globalDotEnv,
    });

    // Second, there is a list of tasks which are conditional.
    Object.entries(rootTurboJson.config.pipeline).forEach(
      ([taskName, taskDefinition]) => {
        let workspaceName;
        let scriptName;

        if (taskName.length === 0) {
          throw new Error("Invalid task name found in turbo.json.");
        }

        // See if there is a workspace name.
        if (taskName.indexOf("#") !== -1) {
          if (taskName.indexOf("#") === taskName.lastIndexOf("#")) {
            [workspaceName, scriptName] = taskName.split("#");
            if (workspaceName.length === 0 || scriptName.length === 0) {
              throw new Error("Invalid task name found in turbo.json.");
            }

            // This task applies to
            workspaceTasks[workspaceName][scriptName] = getEnvContext(
              rootTurboJson.turboConfigPath,
              {
                legacyConfig: taskDefinition.dependsOn,
                env: taskDefinition.env,
                passThroughEnv: taskDefinition.passThroughEnv,
                dotEnv: taskDefinition.dotEnv,
              }
            );
          }
        } else {
          scriptName = taskName;
          globalTasks[scriptName] = getEnvContext(
            rootTurboJson.turboConfigPath,
            {
              legacyConfig: taskDefinition.dependsOn,
              env: taskDefinition.env,
              passThroughEnv: taskDefinition.passThroughEnv,
              dotEnv: taskDefinition.dotEnv,
            }
          );
        }
      }
    );
  }

  // 3. Process the rest of the workspace turbo.json files.
  const workspaceTurboJsons = turboJsons.filter(
    (turboJson) => !turboJson.isRootConfig
  );

  workspaceTurboJsons.forEach((turboJson) => {
    Object.entries(turboJson.config.pipeline).forEach(
      ([taskName, taskDefinition]) => {
        if (taskName.length === 0 || taskName.indexOf("#") !== -1) {
          throw new Error("Invalid task name found in turbo.json.");
        }

        let workspaceName = getWorkspaceName(turboJson.workspacePath);
        let scriptName = taskName;

        workspaceTasks[workspaceName][scriptName] = getEnvContext(
          turboJson.turboConfigPath,
          {
            legacyConfig: taskDefinition.dependsOn,
            env: taskDefinition.env,
            passThroughEnv: taskDefinition.passThroughEnv,
            dotEnv: taskDefinition.dotEnv,
          }
        );
      }
    );
  });

  const calculatedTestContext: TestContext = {
    allowList: allowListTests,
    global: globalEnvContext,
    globalTasks: globalTasks,
    workspaceTasks: workspaceTasks,
  };

  const filePath = getPhysicalFilename();
  const allTurboVars = getEnvVarDependencies({
    cwd,
  });

  // if allTurboVars is null, something went wrong reading from the turbo config
  // (this is different from finding a config with no env vars present, which would
  // return an empty set) - so there is no point continuing if we have nothing to check against
  if (!allTurboVars) {
    // return of {} bails early from a rule check
    return {};
  }

  const globalTurboVars = allTurboVars["//"];
  const hasWorkspaceConfigs = Object.keys(allTurboVars).length > 1;

  // find any workspace configs that match the current file path
  // find workspace config (if any) that match the current file path
  const workspaceKey = Object.keys(allTurboVars).find(
    (workspacePath) => filePath !== "//" && filePath.startsWith(workspacePath)
  );

  let workspaceTurboVars: Set<string> | null = null;
  if (workspaceKey) {
    workspaceTurboVars = allTurboVars[workspaceKey];
  }

  const checkKey = (node: Node, envKey?: string) => {
    if (envKey) {
      let workspaceName = workspaceNameFromFilePath(filePath);
      let configured = checkForInclusion(
        calculatedTestContext,
        workspaceName,
        envKey
      );
      console.log(configured);
    }

    if (
      envKey &&
      !globalTurboVars.has(envKey) &&
      !regexAllowList.some((regex) => regex.test(envKey))
    ) {
      // if we have a workspace config, check that too
      if (workspaceTurboVars && workspaceTurboVars.has(envKey)) {
        return {};
      } else {
        let message = `{{ envKey }} is not listed as a dependency in ${
          hasWorkspaceConfigs ? "root turbo.json" : "turbo.json"
        }`;
        if (workspaceKey && workspaceTurboVars) {
          if (cwd) {
            // if we have a cwd, we can provide a relative path to the workspace config
            message = `{{ envKey }} is not listed as a dependency in the root turbo.json or workspace (${path.relative(
              cwd,
              workspaceKey
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
      }
    }
  };

  const isComputed = (
    node: MemberExpression & Rule.NodeParentExtension
  ): boolean => {
    if ("computed" in node.parent) {
      return node.parent.computed;
    }

    return false;
  };

  return {
    MemberExpression(node) {
      // we only care about complete process env declarations and non-computed keys
      if (
        "name" in node.object &&
        "name" in node.property &&
        !isComputed(node)
      ) {
        const objectName = node.object.name;
        const propertyName = node.property.name;

        // we're doing something with process.env
        if (objectName === "process" && propertyName === "env") {
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
            checkKey(node.parent, node.parent.property?.name);
          }
        }
      }
    },
  };
}

const rule = { create, meta };
export default rule;
