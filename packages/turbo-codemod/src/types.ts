import type { TransformerResults } from "./runner";

export interface Transformer {
  name: string;
  description: string;
  introducedIn: string;
  idempotent?: boolean;
  transformer: (
    args: TransformerArgs
  ) => Promise<TransformerResults> | TransformerResults;
}

export interface TransformerOptions {
  force: boolean;
  dryRun: boolean;
  print: boolean;
}

export interface TransformerArgs {
  root: string;
  options: TransformerOptions;
}

export interface UtilityArgs extends TransformerOptions {
  transformer: string;
  rootPath: string;
}
