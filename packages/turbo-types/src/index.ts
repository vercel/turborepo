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
} from "./types/config-v2";

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
} from "./types/config-v1";

export type { DryRun } from "./types/dry";
