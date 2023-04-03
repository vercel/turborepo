import { CreateCommandOptions } from "../commands/create/types";
import { RepoInfo } from "../utils/examples";
import type { Project, PackageManager } from "@turbo/workspaces";

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
