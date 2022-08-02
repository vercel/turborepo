export interface Pipeline {
  outputs?: Array<string>;
  dependsOn?: Array<string>;
  inputs?: Array<string>;
}

export interface TurboConfig {
  baseBranch?: string;
  globalDependencies?: Array<string>;
  pipeline: Record<string, Pipeline>;
}
