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

export interface MetaJson {
  maintainedByCoreTeam?: string;
  name?: string;
  description?: string;
  featured?: boolean;
  boost?: boolean;
  template?: boolean;
}

export interface TransformResponse {
  // errors should be thrown as instances of TransformError
  result: "not-applicable" | "success";
  name: string;
  metaJson?: MetaJson;
}

export type TransformResult = Promise<TransformResponse>;
