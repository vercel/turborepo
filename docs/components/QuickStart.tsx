import Feature from "./Feature";
import { SparklesIcon, ServerIcon } from "@heroicons/react/outline";
import Link from "next/link";

export const QuickStartArea = () => {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      <Feature
        feature={{
          Icon: SparklesIcon,
          description: (
            <p>
              Want to start from scratch? Check out our guide on{" "}
              <Link href="/docs/getting-started/create-new">
                building a brand-new monorepo with Turborepo
              </Link>
              .
            </p>
          ),
          name: "Create a new monorepo",
        }}
        detailed
      ></Feature>
      <Feature
        feature={{
          Icon: ServerIcon,
          description: (
            <p>
              Turborepo can be incrementally added to any codebase. Check out
              our guide on{" "}
              <Link href="/docs/getting-started/existing-monorepo">
                adding Turborepo to an existing monorepo
              </Link>
              .
            </p>
          ),
          name: "Add to existing monorepo",
        }}
        detailed
      ></Feature>
    </div>
  );
};
