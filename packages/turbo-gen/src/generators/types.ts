import type { Project } from "@turbo/workspaces";
import type { TurboGeneratorOptions } from "../commands/workspace";
import type { CustomGeneratorOptions } from "../commands/run";

export type WorkspaceType = "app" | "package";

export interface TurboGeneratorArguments {
  project: Project;
  opts: TurboGeneratorOptions;
}

export interface CustomGeneratorArguments {
  generator: string | undefined;
  project: Project;
  opts: CustomGeneratorOptions;
}
