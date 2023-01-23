import { TransformerResults } from "./runner";

export type Transformer = {
  name: string;
  value: string;
  introducedIn: string;
  transformer: (args: TransformerArgs) => TransformerResults;
};

export type TransformerOptions = {
  force: boolean;
  dry: boolean;
  print: boolean;
};

export type TransformerArgs = {
  root: string;
  options: TransformerOptions;
};

export interface UtilityArgs extends TransformerOptions {
  transformer: string;
  rootPath: string;
}
