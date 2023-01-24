import { TransformerOptions } from "../../types";

export type TransformCommandArgument = "string" | undefined;

export interface TransformCommandOptions extends TransformerOptions {
  list: boolean;
}
