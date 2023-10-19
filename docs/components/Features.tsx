import React from "react";
import {
  LEGACY_REPO_DOCS_FEATURES,
  LEGACY_REPO_HOME_FEATURES,
} from "../content/legacy-features";
import Feature from "./Feature";

export function HomeFeatures() {
  return (
    <DetailedFeaturesGrid>
      {LEGACY_REPO_HOME_FEATURES.map((feature) => (
        <Feature
          key={feature.name.split(" ").join("-")}
          feature={feature}
          detailed
        />
      ))}
    </DetailedFeaturesGrid>
  );
}

export function DocsFeatures({ detailed = true }: { detailed?: boolean }) {
  return (
    <div className="grid grid-cols-2 gap-6 my-12 sm:grid-cols-3 ">
      {LEGACY_REPO_DOCS_FEATURES.map((feature) => (
        <Feature
          key={feature.name.split(" ").join("-")}
          feature={feature}
          detailed={detailed}
        />
      ))}
    </div>
  );
}

export function DetailedFeaturesGrid({
  children,
}: {
  children?: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:grid-cols-3 lg:gap-x-8 lg:gap-y-12">
      {children}
    </div>
  );
}
