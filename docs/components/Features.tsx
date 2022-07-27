import Feature from "./Feature";
import { DOCS_FEATURES, HOME_FEATURES } from "../content/features";
import type { Feature as FeatureType } from "../content/features";

export default function Features({
  page = "home",
  detailed = true,
}: {
  page?: FeatureType["page"];
  detailed?: boolean;
}) {
  if (page === "docs") {
    return (
      <div className="grid grid-cols-2 gap-6 my-12 sm:grid-cols-3 ">
        {DOCS_FEATURES.map((feature) => (
          <Feature
            key={feature.name.split(" ").join("-")}
            feature={feature}
            detailed={detailed}
          />
        ))}
      </div>
    );
  } else {
    return (
      <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:grid-cols-3 lg:gap-x-8 lg:gap-y-12">
        {HOME_FEATURES.map((feature) => (
          <Feature
            key={feature.name.split(" ").join("-")}
            feature={feature}
            detailed
          />
        ))}
      </div>
    );
  }
}
