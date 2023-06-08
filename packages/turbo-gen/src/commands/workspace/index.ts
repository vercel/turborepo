import { logger } from "@turbo/utils";
import { getProject } from "../../utils/getProject";
import { copy, empty } from "../../generators";
import { TurboGeneratorOptions, WorkspaceType } from "../../generators/types";

export interface TurboGeneratorCLIOptions {
  name?: string;
  // default to true
  empty: boolean;
  copy?: string | boolean;
  destination?: string;
  type?: WorkspaceType;
  root?: string;
  examplePath?: string;
  // defaults to false
  showAllDependencies: boolean;
}

// convert CLI options to generator options
function parse(opts: TurboGeneratorCLIOptions): TurboGeneratorOptions {
  const { empty, copy, ...rest } = opts;
  const method = copy === true || typeof copy === "string" ? "copy" : "empty";
  const source = typeof copy === "string" ? copy : "";
  const sourceType =
    typeof copy === "string" && copy.startsWith("https://")
      ? "external"
      : "internal";

  return {
    method,
    copy: {
      type: sourceType,
      source,
    },
    ...rest,
  };
}

/**
 * Adds a new (blank, or copied) workspace to the project
 */
export async function workspace(opts: TurboGeneratorCLIOptions) {
  const project = await getProject(opts);
  const generatorOpts = parse(opts);

  console.log();
  const args = {
    project,
    opts: generatorOpts,
  };

  if (generatorOpts.method === "copy") {
    if (generatorOpts.copy.type === "external") {
      logger.info(`Copy a remote workspace from ${generatorOpts.copy.source}`);
    } else {
      logger.info(`Copy an existing workspace from "${project.name}"`);
    }
    console.log();
    await copy(args);
  } else {
    logger.info(`Add an empty workspace to "${project.name}"`);
    console.log();
    await empty(args);
  }
}
