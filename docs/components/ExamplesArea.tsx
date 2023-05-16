import { useSSG } from "nextra/ssg";
import { ExampleCard } from "./ExamplesCard";

export const ExamplesArea = ({
  filter = "featured",
}: {
  filter: "featured" | "all";
}) => {
  const { examples } = useSSG();

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:mt-16 mt-12 gap-x-6 gap-y-12  lg:gap-x-8 lg:gap-y-12">
      {examples
        .filter(({ featured }) => (filter === "featured" ? featured : true))
        // sort templates to the top
        .sort((a) => (a.template ? -1 : 1))
        .map(({ name, description, slug, featured, template }) => (
          <ExampleCard
            key={name}
            name={name}
            description={description}
            slug={slug}
            template={template}
          />
        ))}
    </div>
  );
};
