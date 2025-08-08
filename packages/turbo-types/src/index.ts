import type { Framework as FW } from "./types/frameworks.js";
// @ts-ignore - JSON imports work differently in ESM vs CJS
import frameworksJson from "./json/frameworks.json";

export const frameworks = frameworksJson as Array<Framework>;
export type Framework = FW;
export type { FrameworkStrategy } from "./types/frameworks.js";

export {
  type BaseSchema,
  type BaseSchema as BaseSchemaV2,
  type EnvWildcard,
  type EnvWildcard as EnvWildcardV2,
  type OutputLogs as OutputLogsV2,
  type Pipeline,
  type Pipeline as PipelineV2,
  type RemoteCache,
  type RemoteCache as RemoteCacheV2,
  type RootSchema,
  type RootSchema as RootSchemaV2,
  type Schema,
  type Schema as SchemaV2,
  type UI,
  type UI as UIV2,
  type WorkspaceSchema,
  type WorkspaceSchema as WorkspaceSchemaV2,
  isRootSchemaV2,
  isWorkspaceSchemaV2,
} from "./types/config-v2.js";

export {
  type BaseSchemaV1,
  type EnvWildcardV1,
  type OutputModeV1,
  type PipelineV1,
  type RemoteCacheV1,
  type RootSchemaV1,
  type SchemaV1,
  type WorkspaceSchemaV1,
  isRootSchemaV1,
  isWorkspaceSchemaV1,
} from "./types/config-v1.js";

export type { DryRun } from "./types/dry.js";
