import { transform as packageManagerTransform } from "./package-manager";
import { transform as internalTransform } from "./internal-example";
import { transform as gitIgnoreTransform } from "./git-ignore";
import type { TransformInput, TransformResult } from "./types";

/**
 * In the future, we may want to support sourcing additional transforms from the templates themselves.
 */
export const transforms: Array<(args: TransformInput) => TransformResult> = [
  internalTransform,
  gitIgnoreTransform,
  packageManagerTransform,
];
