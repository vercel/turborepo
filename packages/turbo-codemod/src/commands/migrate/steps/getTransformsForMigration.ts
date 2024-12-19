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
  const fromMajor = fromVersion.split(".")[0];
  // if migrating "from" to "to" spans a major, floor "from" to ensure all required codemods are run
  const isMajorBump = fromMajor !== toVersion.split(".")[0];
  const resolvedFromVersion = isMajorBump ? `${fromMajor}.0.0` : fromVersion;
  const transforms = loadTransformers().filter((transformer) => {
    const inOriginalRange =
      gt(transformer.introducedIn, fromVersion) &&
      lte(transformer.introducedIn, toVersion);
    // If a transform is only in the expanded range, then we should only perform it
    // if it is idempotent.
    const idempotentAndInExpandedRange =
      (transformer.idempotent ?? true) &&
      gt(transformer.introducedIn, resolvedFromVersion) &&
      lte(transformer.introducedIn, toVersion);
    return inOriginalRange || idempotentAndInExpandedRange;
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
