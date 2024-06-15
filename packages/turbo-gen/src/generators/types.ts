import type { Project } from "@turbo/workspaces";
import type { TurboGeneratorCLIOptions } from "../commands/workspace";
import type { CustomGeneratorCLIOptions } from "../commands/run";

export type WorkspaceType = "app" | "package";
export interface CopyData {
  type: "internal" | "external";
  source: string;
}

export type TurboGeneratorOptions = Omit<
  TurboGeneratorCLIOptions,
  "copy" | "empty"
> & {
  copy: CopyData;
  method: "copy" | "empty";
};

export interface TurboGeneratorArguments {
  project: Project;
  opts: TurboGeneratorOptions;
}

export interface CustomGeneratorArguments {
  generator: string | undefined;
  project: Project;
  opts: CustomGeneratorCLIOptions;
}
