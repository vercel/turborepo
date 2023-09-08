import type { TransformerResults } from "./runner";

export interface Transformer {
  name: string;
  description: string;
  introducedIn: string;
  transformer: (
    args: TransformerArgs
  ) => Promise<TransformerResults> | TransformerResults;
}

export interface TransformerOptions {
  force: boolean;
  dry: boolean;
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
