import type { Workspace } from "@turbo/workspaces";
import type { TurboGeneratorArguments } from "../generators/types";
import * as prompts from "../commands/workspace/prompts";
import { getWorkspaceList } from "./getWorkspaceList";

export async function gatherAddRequirements({
  project,
  opts,
}: TurboGeneratorArguments) {
  let source: Workspace | undefined;

  // suggestion for the name based on the (optional) example path
  const suggestion =
    opts.method === "copy" && opts.copy.type === "external"
      ? opts.examplePath?.split("/").pop() || opts.copy.source.split("/").pop()
      : undefined;

  const { answer: type } = await prompts.type({
    override: opts.type,
    message:
      opts.method === "copy" && opts.copy.source === "external" && suggestion
        ? `What type of workspace should "${suggestion}" be created as?`
        : undefined,
  });

  const { answer: name } = await prompts.name({
    override: opts.name,
    workspaceType: type,
    suggestion,
  });

  // if we're copying an internal workspace, we need to know which one
  if (opts.method === "copy" && opts.copy.type === "internal") {
    const { answer } = await prompts.source({
      override: opts.copy.source,
      workspaces: getWorkspaceList({ project, type }),
      workspaceName: name,
    });
    source = answer;
  }

  const location = await prompts.location({
    workspaceType: type,
    workspaceName: name,
    project,
    destination: opts.destination,
  });

  const dependencies = await prompts.dependencies({
    workspaceName: name,
    project,
    workspaceSource: source,
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
