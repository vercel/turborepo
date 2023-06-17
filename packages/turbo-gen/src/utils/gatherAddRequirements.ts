import { Workspace } from "@turbo/workspaces";
import type { TurboGeneratorArguments } from "../generators/types";
import * as prompts from "../commands/workspace/prompts";
import { getWorkspaceList } from "./getWorkspaceList";

export async function gatherAddRequirements({
  project,
  opts,
}: TurboGeneratorArguments) {
  let source: Workspace | undefined = undefined;
  // suggestion for the name based on the (optional) example path
  const suggestion =
    opts.copy.type === "external"
      ? opts.examplePath?.split("/").pop() || opts.copy.source.split("/").pop()
      : undefined;

  const { answer: type } = await prompts.type({
    override: opts.type,
    message:
      opts.copy.source === "external"
        ? `What type of workspace should "${suggestion}" be created as?`
        : undefined,
  });

  const { answer: name } = await prompts.name({
    override: opts.name,
    type,
    suggestion,
  });

  if (opts.copy.type === "internal") {
    const { answer } = await prompts.source({
      override: opts.copy.source,
      workspaces: getWorkspaceList({ project, type }),
      name,
    });
    source = answer;
  }
  const location = await prompts.location({
    type,
    name,
    project,
    destination: opts.destination,
  });

  const dependencies = await prompts.dependencies({
    name,
    project,
    source,
    showAllDependencies: opts.showAllDependencies,
  });

  return {
    type,
    name,
    location,
    source,
    dependencies,
  };
}
