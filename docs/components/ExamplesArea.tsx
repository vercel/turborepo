import { useSSG } from "nextra/ssg";
import { DetailedFeatureLink } from "./Feature";
import { GitHubIcon } from "./Icons";

export const ExamplesArea = ({
  filter = "featured",
}: {
  filter: "featured" | "all";
}) => {
  const { examples } = useSSG();

  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      {examples
        .filter(({ featured }) => (filter === "featured" ? featured : true))
        .map(({ name, description, slug }) => (
          <DetailedFeatureLink
            key={name}
            feature={{
              Icon: GitHubIcon,
              description,
              name,
            }}
            target="_blank"
            href={`https://github.com/vercel/turbo/tree/main/examples/${slug}`}
          />
        ))}
    </div>
  );
};
