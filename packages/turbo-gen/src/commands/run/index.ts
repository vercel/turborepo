import { logger } from "@turbo/utils";
import { getProject } from "../../utils/getProject";
import { custom } from "../../generators";

export interface CustomGeneratorCLIOptions {
  config?: string;
  root?: string;
  args?: Array<string>;
}

/**
 * Runs a custom generator (optionally specified by "generator")
 */
export async function run(
  generator: string | undefined,
  opts: CustomGeneratorCLIOptions
) {
  const project = await getProject(opts);

  logger.log();
  logger.info(`Modify "${project.name}" using custom generators`);
  logger.log();

  await custom({ generator, project, opts });
}
