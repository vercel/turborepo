import { useSSG } from "nextra/ssg";
import { ExampleCard } from "./ExamplesCard";

interface Example {
  name: string;
  description: string;
  slug: string;
  template?: string;
  featured?: boolean;
  boost?: boolean;
}

const ExamplesGroup = ({ examples }: { examples: Array<Example> }) => {
  return (
    <>
      {examples.map(({ name, description, slug, template }) => (
        <ExampleCard
          key={name}
          name={name}
          description={description}
          slug={slug}
          template={template}
        />
      ))}
    </>
  );
};

export const ExamplesArea = ({
  filter = "featured",
}: {
  filter: "featured" | "all";
}) => {
  const { examples }: { examples: Array<Example> } = useSSG();

  const sortedExamples = examples
    .filter(({ featured }) => (filter === "featured" ? featured : true))
    .sort((a, b) => a.name.localeCompare(b.name));

  const withBoost = [];
  const withTemplate = [];
  const withoutTemplate = [];
  sortedExamples.forEach((e) => {
    if (e.boost) {
      withBoost.push(e);
    } else if (e.template) {
      withTemplate.push(e);
    } else {
      withoutTemplate.push(e);
    }
  });

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:mt-16 mt-12 gap-x-6 gap-y-12  lg:gap-x-8 lg:gap-y-12">
      {/* Render examples in three groups -
        1. Examples that have been explicitly boosted
        2. Examples with Vercel templates
        3. All others
      */}
      <ExamplesGroup examples={withBoost} />
      <ExamplesGroup examples={withTemplate} />
      <ExamplesGroup examples={withoutTemplate} />
    </div>
  );
};
