import { PACK_HOME_FEATURES } from "../../../content/features";
import { FadeIn } from "./FadeIn";
import { SectionHeader, SectionSubtext } from "./Headings";
import { PackFeature } from "./PackFeature";

export function PackFeatures() {
  return (
    <section className="font-sans relative px-6 pb-16 md:pb-24 lg:pb-32 gap-9 lg:gap-14 items-center flex flex-col">
      <FadeIn className="flex flex-col gap-5 md:gap-6">
        <SectionHeader>Why Turbopack?</SectionHeader>
        <SectionSubtext>
          With incremental behavior and adaptable bundling strategies, Turbopack
          provides a fast and flexible development experience for apps of any
          size.
        </SectionSubtext>
      </FadeIn>
      <div className="grid grid-cols-1 gap-x-4 gap-y-4 sm:grid-cols-2 lg:grid-cols-3 lg:gap-x-6 lg:gap-y-6 max-w-[1200px]">
        {PACK_HOME_FEATURES.map((feature) => (
          <FadeIn
            className="flex"
            key={feature.name.replace(/\s+/g, "-").toLowerCase()}
          >
            <PackFeature
              name={feature.name}
              description={feature.description}
              iconDark={feature.IconDark}
              iconLight={feature.IconLight}
            />
          </FadeIn>
        ))}
      </div>
    </section>
  );
}
