import { gt, lte } from "semver";

import loadTransformers from "../../../utils/loadTransformers";
import type { Transformer } from "../../../types";

/**
  Returns all transformers introduced after fromVersion, but before or equal to toVersion
**/
function getTransformsForMigration({
  fromVersion,
  toVersion,
}: {
  fromVersion: string;
  toVersion: string;
}): Array<Transformer> {
  const transforms = loadTransformers();
  return transforms.filter((transformer) => {
    return (
      gt(transformer.introducedIn, fromVersion) &&
      lte(transformer.introducedIn, toVersion)
    );
  });
}

export default getTransformsForMigration;
