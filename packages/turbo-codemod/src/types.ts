export interface Flags {
  dry: boolean;
  force: boolean;
  print: boolean;
}

export interface Pipeline {
  outputs?: Array<string>;
  dependsOn?: Array<string>;
  env?: Array<string>;
  inputs?: Array<string>;
  cache?: boolean;
}

export interface TurboConfig {
  $schema?: string;
  globalDependencies?: Array<string>;
  env?: Array<string>;
  pipeline?: Record<string, Pipeline>;
}
