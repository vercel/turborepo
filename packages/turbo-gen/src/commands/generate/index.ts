import { logger } from "@turbo/utils";
import { getProject } from "../../utils/getProject";
import { custom } from "../../generators";

export interface CustomGeneratorOptions {
  config?: string;
  root?: string;
  args?: Array<string>;
}

export async function generate(
  generator: string | undefined,
  opts: CustomGeneratorOptions
) {
  const project = await getProject(opts);

  console.log();
  logger.info(`Modify ${project.name} using custom generators`);
  console.log();

  await custom({ generator, project, opts });
}
