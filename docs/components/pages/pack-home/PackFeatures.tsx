import { PACK_HOME_FEATURES } from "../../../content/features";
import { FeaturesBento } from "../home-shared/FeaturesBento";

export function PackFeatures() {
  return (
    <FeaturesBento
      body="With incremental behavior and adaptable bundling strategies, Turbopack provides a fast and flexible development experience for apps of any size."
      features={PACK_HOME_FEATURES}
      header="Why Turbopack?"
    />
  );
}
