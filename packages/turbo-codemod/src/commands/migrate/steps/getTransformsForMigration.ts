import { gt, lte, eq } from "semver";
import { loadTransformers } from "../../../utils/loadTransformers";
import type { Transformer } from "../../../types";

/**
 * Returns all transformers introduced after fromVersion, but before or equal to toVersion
 **/
export function getTransformsForMigration({
  fromVersion,
  toVersion,
}: {
  fromVersion: string;
  toVersion: string;
}): Array<Transformer> {
  const transforms = loadTransformers().filter((transformer) => {
    return (
      gt(transformer.introducedIn, fromVersion) &&
      lte(transformer.introducedIn, toVersion)
    );
  });

  // Sort the transforms from oldest (1.0) to newest (1.10).
  transforms.sort((a, b) => {
    if (gt(a.introducedIn, b.introducedIn)) {
      return 1;
    } else if (eq(a.introducedIn, b.introducedIn)) {
      return 0;
    }
    return -1;
  });

  return transforms;
}
