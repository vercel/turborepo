type ChunkPath = string;
type ModuleId = string;

interface Exports {
  __esModule?: boolean;

  [key: string]: any;
}

interface Module {
  exports: Exports;
  loaded: boolean;
  id: ModuleId;
  children: ModuleId[];
  parents: ModuleId[];
  interopNamespace?: EsmInteropNamespace;
}

enum SourceType {
  /**
   * The module was instantiated because it was included in an evaluated chunk's
   * runtime.
   */
  Runtime = 0,
  /**
   * The module was instantiated because a parent module imported it.
   */
  Parent = 1,
}

type SourceInfo =
  | {
      type: SourceType.Runtime;
      chunkPath: ChunkPath;
    }
  | {
      type: SourceType.Parent;
      parentId: ModuleId;
    };

type ModuleCache = Record<ModuleId, Module>;

type CommonJsRequire = (moduleId: ModuleId) => Exports;

export type EsmInteropNamespace = Record<string, any>;
type EsmImport = (
  moduleId: ModuleId,
  allowExportDefault: boolean
) => EsmInteropNamespace;
type EsmExport = (exportGetters: Record<string, () => any>) => void;
type ExportValue = (value: any) => void;

type LoadChunk = (chunkPath: ChunkPath) => Promise<any> | undefined;

interface TurbopackContext {
  e: Module["exports"];
  r: CommonJsRequire;
  i: EsmImport;
  s: EsmExport;
  v: ExportValue;
  m: Module;
  c: ModuleCache;
  l: LoadChunk;
  p: Partial<NodeJS.Process> & Pick<NodeJS.Process, "env">;
}

type ModuleFactory = (
  this: Module["exports"],
  context: TurbopackContext
) => undefined;

// string encoding of a module factory (used in hmr updates)
type ModuleFactoryString = string;

interface RuntimeBackend {
  registerChunk: (chunkPath: ChunkPath) => void;
  loadChunk: (chunkPath: ChunkPath, source: SourceInfo) => Promise<void>;
}

export type InstantiateRuntimeModule = (
  moduleId: ModuleId,
  chunkPath: ChunkPath
) => Module;

declare global {
  const RUNTIME_PUBLIC_PATH: string;
}
