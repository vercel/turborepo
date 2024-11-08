import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type { WorkspaceConfig } from "@turbo/utils";
import { getWorkspaceConfigs } from "@turbo/utils";
import type { PipelineV1, RootSchemaV1, RootSchemaV2 } from "@turbo/types";
import { forEachTaskDef } from "@turbo/utils/src/getTurboConfigs";
import { dotEnv } from "./dotenv-processing";
import { wildcardTests } from "./wildcard-processing";

interface EnvironmentConfig {
  legacyConfig: Array<string>;
  env: Array<string>;
  passThroughEnv: Array<string> | null;
  dotEnv: DotEnvConfig | null;
}

type EnvVar = string;
type EnvTest = (variable: EnvVar) => boolean;
interface EnvironmentTest {
  legacyConfig: EnvTest;
  env: EnvTest;
  passThroughEnv: EnvTest;
  dotEnv: EnvTest;
}

interface DotEnvConfig {
  filePaths: Array<string>;
  hashes: Record<string, string | null>;
}

export interface ProjectKey {
  global: EnvironmentConfig;
  globalTasks: Record<string, EnvironmentConfig>;
  workspaceTasks: Record<string, Record<string, EnvironmentConfig>>;
}

interface ProjectTests {
  global: EnvironmentTest;
  globalTasks: Record<string, EnvironmentTest>;
  workspaceTasks: Record<string, Record<string, EnvironmentTest>>;
}

// Process inputs for `EnvironmentConfig`s

function processLegacyConfig(
  legacyConfig: Array<string> | undefined
): Array<string> {
  if (!legacyConfig) {
    return [];
  }

  const processed = legacyConfig
    // filter for env vars
    .filter((dep) => dep.startsWith("$"))
    // remove leading $
    .map((variable) => variable.slice(1));

  // After processing length is 0, 1, or more than 1.
  switch (processed.length) {
    case 0:
      return [];
    case 1:
      return processed;
    default:
      return [...new Set(processed)].sort();
  }
}

function processEnv(env: Array<string> | undefined): Array<string> {
  if (!env) {
    return [];
  }

  switch (env.length) {
    case 0:
      return [];
    case 1:
      return [env[0]];
    default:
      return [...new Set(env)].sort();
  }
}

function processPassThroughEnv(
  passThroughEnv: Array<string> | null | undefined
): Array<string> | null {
  if (!passThroughEnv) {
    return null;
  }

  switch (passThroughEnv.length) {
    case 0:
      return [];
    case 1:
      return [passThroughEnv[0]];
    default:
      return [...new Set(passThroughEnv)].sort();
  }
}

function processDotEnv(
  workspacePath: string,
  filePaths: Array<string> | null | undefined
): DotEnvConfig | null {
  if (!filePaths) {
    return null;
  }

  const hashEntries: Array<[string, string]> = [];
  filePaths.reduce((accumulator, filePath) => {
    const hash = crypto.createHash("sha1");
    try {
      const fileContents = fs.readFileSync(path.join(workspacePath, filePath));
      hash.update(fileContents);
      accumulator.push([filePath, hash.digest("hex")]);
    } catch (_) {
      // ignore
    }

    return accumulator;
  }, hashEntries);

  return {
    filePaths,
    hashes: Object.fromEntries(hashEntries),
  };
}

// Generate `EnvironmentConfig`s

function processGlobal(
  workspacePath: string,
  schema: RootSchemaV1 | RootSchemaV2
): EnvironmentConfig {
  return {
    legacyConfig: processLegacyConfig(schema.globalDependencies),
    env: processEnv(schema.globalEnv),
    passThroughEnv: processPassThroughEnv(schema.globalPassThroughEnv),
    dotEnv: processDotEnv(
      workspacePath,
      "globalDotEnv" in schema ? schema.globalDotEnv : undefined
    ),
  };
}

function processTask(
  workspacePath: string,
  task: PipelineV1
): EnvironmentConfig {
  return {
    legacyConfig: processLegacyConfig(task.dependsOn),
    env: processEnv(task.env),
    passThroughEnv: processPassThroughEnv(task.passThroughEnv),
    dotEnv: processDotEnv(workspacePath, task.dotEnv),
  };
}

const TEST_FALSE = (_: string): boolean => false;
function generateEnvironmentTest(
  config: EnvironmentConfig,
  workspacePath: string | undefined
): EnvironmentTest {
  const output: EnvironmentTest = {
    legacyConfig: TEST_FALSE,
    env: TEST_FALSE,
    passThroughEnv: TEST_FALSE,
    dotEnv: TEST_FALSE,
  };

  if (config.legacyConfig.length > 0) {
    const dependsOnEnvSet = new Set(config.legacyConfig);
    output.legacyConfig = (variable: EnvVar) => dependsOnEnvSet.has(variable);
  }

  if (config.env.length > 0) {
    const testables = wildcardTests(config.env);
    output.env = (variable: EnvVar) => {
      return (
        testables.inclusions.test(variable) &&
        !testables.exclusions.test(variable)
      );
    };
  }

  // c. Check the passThroughEnv configuration.
  if (config.passThroughEnv && config.passThroughEnv.length > 0) {
    const testables = wildcardTests(config.passThroughEnv);
    output.passThroughEnv = (variable: EnvVar) => {
      return (
        testables.inclusions.test(variable) &&
        !testables.exclusions.test(variable)
      );
    };
  }

  // d. Check to see if the variable is accounted for by dotEnv.
  if (config.dotEnv && config.dotEnv.filePaths.length > 0) {
    const dotEnvEnvSet = dotEnv(workspacePath, config.dotEnv);
    output.dotEnv = (variable: EnvVar) => dotEnvEnvSet.has(variable);
  }

  return output;
}

function environmentTestArray(envContext: EnvironmentTest) {
  return [
    envContext.legacyConfig,
    envContext.env,
    envContext.passThroughEnv,
    envContext.dotEnv,
  ];
}

// Identify where to store `EnvironmentConfig`s

function getTaskAddress(taskName: string): {
  workspaceName: string | null;
  scriptName: string;
} {
  // Somehow empty. Error.
  if (taskName.length === 0) {
    throw new Error("Invalid task name found in turbo.json.");
  }

  const firstIndexOf = taskName.indexOf("#");

  // Something like "build"
  if (firstIndexOf === -1) {
    return {
      workspaceName: null,
      scriptName: taskName,
    };
  }

  // Something like "what#are#you#doing"
  if (firstIndexOf !== taskName.lastIndexOf("#")) {
    throw new Error("Invalid task name found in turbo.json.");
  }

  const [workspaceName, scriptName] = taskName.split("#");

  return {
    workspaceName,
    scriptName,
  };
}

export function getWorkspaceFromFilePath(
  projectWorkspaces: Array<WorkspaceConfig>,
  filePath: string
): WorkspaceConfig | null {
  const possibleWorkspaces = projectWorkspaces
    .filter((projectWorkspace) =>
      filePath.startsWith(projectWorkspace.workspacePath)
    )
    .sort((a, b) => {
      if (a.workspacePath > b.workspacePath) {
        return -1;
      } else if (a === b) {
        return 0;
      }
      return 1;
    });

  if (possibleWorkspaces.length > 0) {
    return possibleWorkspaces[0];
  }

  return null;
}

// Driver

export class Project {
  _key: ProjectKey;
  _test: ProjectTests;

  cwd: string | undefined;
  allConfigs: Array<WorkspaceConfig>;
  projectRoot: WorkspaceConfig | undefined;
  projectWorkspaces: Array<WorkspaceConfig>;

  constructor(cwd: string | undefined) {
    this.cwd = cwd;
    this.allConfigs = getWorkspaceConfigs(cwd);
    this.projectRoot = this.allConfigs.find(
      (workspaceConfig) => workspaceConfig.isWorkspaceRoot
    );
    this.projectWorkspaces = this.allConfigs.filter(
      (workspaceConfig) => !workspaceConfig.isWorkspaceRoot
    );

    this._key = this.generateKey();
    this._test = this.generateTestConfig();
  }

  valid(): boolean {
    return this.allConfigs.length > 0;
  }

  generateKey(): ProjectKey {
    let global: EnvironmentConfig = {
      legacyConfig: [],
      env: [],
      passThroughEnv: null,
      dotEnv: null,
    };
    const globalTasks: Record<string, EnvironmentConfig> = {};
    const workspaceTasks: Record<
      string,
      Record<string, EnvironmentConfig>
    > = {};

    if (this.projectRoot?.turboConfig && !("extends" in this.projectRoot)) {
      const rootTurboJson = this.projectRoot;

      global = processGlobal(
        this.projectRoot.workspacePath,
        this.projectRoot.turboConfig
      );

      forEachTaskDef(
        this.projectRoot.turboConfig,
        ([taskName, taskDefinition]) => {
          const { workspaceName, scriptName } = getTaskAddress(taskName);
          if (workspaceName) {
            workspaceTasks[workspaceName] =
              workspaceName in workspaceTasks
                ? workspaceTasks[workspaceName]
                : {};
            workspaceTasks[workspaceName][scriptName] = processTask(
              rootTurboJson.workspacePath,
              taskDefinition
            );
          } else {
            globalTasks[scriptName] = processTask(
              rootTurboJson.workspacePath,
              taskDefinition
            );
          }
        }
      );
    }

    this.projectWorkspaces.forEach((projectWorkspace) => {
      if (!projectWorkspace.turboConfig) {
        return;
      }

      forEachTaskDef(
        projectWorkspace.turboConfig,
        ([taskName, taskDefinition]) => {
          const { workspaceName: erroneousWorkspaceName, scriptName } =
            getTaskAddress(taskName);
          if (erroneousWorkspaceName) {
            throw new Error(
              "May not specify workspace name in non-root turbo.json"
            );
          }

          const workspaceName = projectWorkspace.workspaceName;
          workspaceTasks[workspaceName] =
            workspaceName in workspaceTasks
              ? workspaceTasks[workspaceName]
              : {};
          workspaceTasks[workspaceName][scriptName] = processTask(
            projectWorkspace.workspacePath,
            taskDefinition
          );
        }
      );
    });

    return {
      global,
      globalTasks,
      workspaceTasks,
    };
  }

  getWorkspacePath(workspaceName: string): string | undefined {
    return this.projectWorkspaces.find(
      (workspaceConfig) => workspaceConfig.workspaceName === workspaceName
    )?.workspacePath;
  }

  generateTestConfig(): ProjectTests {
    return {
      global: generateEnvironmentTest(
        this._key.global,
        this.projectRoot?.workspacePath
      ),
      globalTasks: Object.fromEntries(
        Object.entries(this._key.globalTasks).map(([script, config]) => {
          return [
            script,
            generateEnvironmentTest(config, this.projectRoot?.workspacePath),
          ];
        })
      ),
      workspaceTasks: Object.fromEntries(
        Object.entries(this._key.workspaceTasks).map(
          ([workspace, taskConfigs]) => {
            const workspacePath = this.getWorkspacePath(workspace);
            return [
              workspace,
              Object.fromEntries(
                Object.entries(taskConfigs).map(([script, config]) => {
                  return [
                    script,
                    generateEnvironmentTest(config, workspacePath),
                  ];
                })
              ),
            ];
          }
        )
      ),
    };
  }

  key() {
    return this._key;
  }

  test(workspaceName: string | undefined, envVar: string) {
    const tests = [
      environmentTestArray(this._test.global),
      ...Object.values(this._test.globalTasks).map((context) =>
        environmentTestArray(context)
      ),
    ];

    if (workspaceName && workspaceName in this._test.workspaceTasks) {
      tests.push(
        ...Object.values(this._test.workspaceTasks[workspaceName]).map(
          (context) => environmentTestArray(context)
        )
      );
    }

    return tests.flat().some((test) => test(envVar));
  }
}
