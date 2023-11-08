import type { RepoInfo, PackageManager } from "@turbo/utils";
import type { Project } from "@turbo/workspaces";
import type { CreateCommandOptions } from "../commands/create/types";

export interface TransformInput {
  example: {
    repo: RepoInfo | undefined;
    name: string;
  };
  project: Project;
  prompts: {
    projectName: string;
    root: string;
    packageManager:
      | {
          name: PackageManager;
          version: string | undefined;
        }
      | undefined;
  };
  opts: CreateCommandOptions;
}

export interface TransformResponse {
  // errors should be thrown as instances of TransformError
  result: "not-applicable" | "success";
  name: string;
}

export type TransformResult = Promise<TransformResponse>;
