import { Workspace } from "@turbo/workspaces";
import type { TurboGeneratorArguments } from "../generators/types";
import * as prompts from "../commands/add/prompts";
import { getWorkspaceList } from "./getWorkspaceList";

export async function gatherAddRequirements({
  project,
  opts,
}: TurboGeneratorArguments) {
  let source: Workspace | undefined = undefined;
  const { answer: what } = await prompts.what({ override: opts.what });

  // suggestion for the name based on the (optional) example path
  const suggestion =
    opts.examplePath?.split("/").pop() || opts.example?.split("/").pop();

  const { answer: name } = await prompts.name({
    override: opts.name,
    what,
    suggestion,
  });
  if (opts.copy && !opts.example) {
    const { answer } = await prompts.source({
      workspaces: getWorkspaceList({ project, what }),
      name,
    });
    source = answer;
  }
  const location = await prompts.location({
    what,
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
    what,
    name,
    location,
    source,
    dependencies,
  };
}
