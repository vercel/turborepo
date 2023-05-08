import { logger } from "@turbo/utils";
import { getProject } from "../../utils/getProject";
import { copy, empty } from "../../generators";
import { WorkspaceType } from "../../generators/types";

export interface TurboGeneratorOptions {
  name?: string;
  // default to true
  empty: boolean;
  copy?: boolean;
  destination?: string;
  what?: WorkspaceType;
  root?: string;
  example?: string;
  examplePath?: string;
  // defaults to false
  showAllDependencies: boolean;
}

export async function add(opts: TurboGeneratorOptions) {
  const project = await getProject(opts);

  console.log();
  const args = { project, opts };
  if (opts.copy) {
    if (opts.example) {
      logger.info(`Copy a remote workspace from ${opts.example}`);
    } else {
      logger.info(`Copy an existing workspace from ${project.name}`);
    }
    console.log();
    await copy(args);
  } else {
    logger.info(`Add an empty workspace to ${project.name}`);
    console.log();
    await empty(args);
  }
}
