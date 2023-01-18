import { Change } from "diff";

export interface FileResult {
  action: "skipped" | "modified" | "unchanged" | "error";
  error?: Error;
  additions: number;
  deletions: number;
}

export interface FileTransformArgs extends ModifyFileArgs {
  rootPath: string;
}

export interface ModifyFileArgs {
  filePath: string;
  before?: string | object;
  after?: string | object;
  error?: Error;
}

export interface AbortTransformArgs {
  reason: string;
  changes?: Record<string, FileResult>;
}

export interface LogFileArgs {
  diff?: boolean;
}

export type FileWriter = (filePath: string, contents: string | object) => void;

export type FileDiffer = (
  before: string | object,
  after: string | object
) => Array<Change>;

export interface TransformerResults {
  fatalError?: Error;
  changes: Record<string, FileResult>;
}
