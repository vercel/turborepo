import { ServerIcon, SparklesIcon } from "@heroicons/react/outline";
import { DetailedFeatureLink } from "./Feature";

export const QuickStartArea = () => {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      <DetailedFeatureLink
        feature={{
          Icon: SparklesIcon,
          description: (
            <p>
              Want to start from scratch? Check out our guide on building a
              brand-new monorepo with Turborepo.
            </p>
          ),
          name: "Create a new monorepo",
        }}
        href="/docs/getting-started/create-new"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: ServerIcon,
          description: (
            <p>
              Turborepo can be incrementally added to any codebase. Check out
              our guide on adding Turborepo to an existing monorepo.
            </p>
          ),
          name: "Add to existing monorepo",
        }}
        href="/docs/getting-started/existing-monorepo"
      ></DetailedFeatureLink>
    </div>
  );
};
