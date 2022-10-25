import type { Features } from "../../../content/features";
import { FadeIn } from "./FadeIn";
import { SectionHeader, SectionSubtext } from "./Headings";
import { FeatureBox } from "./FeatureBox";

export function FeaturesBento({
  header,
  body,
  features,
}: {
  header: string;
  body: string;
  features: Features;
}) {
  return (
    <section className="relative flex flex-col items-center px-6 pb-16 font-sans md:pb-24 lg:pb-32 gap-9 lg:gap-14">
      <FadeIn className="flex flex-col items-center gap-5 md:gap-6">
        <SectionHeader>{header}</SectionHeader>
        <SectionSubtext>{body}</SectionSubtext>
      </FadeIn>
      <div className="grid grid-cols-1 gap-x-4 gap-y-4 sm:grid-cols-2 lg:grid-cols-3 lg:gap-x-6 lg:gap-y-6 max-w-[1200px]">
        {features.map((feature) => (
          <FadeIn
            className="flex"
            key={feature.name.replace(/\s+/g, "-").toLowerCase()}
          >
            <FeatureBox
              name={feature.name}
              description={feature.description}
              iconDark={feature.iconDark}
              iconLight={feature.iconLight}
            />
          </FadeIn>
        ))}
      </div>
    </section>
  );
}
