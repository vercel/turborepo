import {
  CloudDownloadIcon,
  CloudUploadIcon,
  LightningBoltIcon,
  ServerIcon,
  SparklesIcon,
} from "@heroicons/react/outline";
import { DetailedFeatureLink } from "./Feature";

export const QuickStartArea = () => {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      <DetailedFeatureLink
        feature={{
          Icon: SparklesIcon,
          description: `Build a brand-new monorepo powered by Turborepo.`,
          name: "Create a new monorepo",
        }}
        href="/docs/getting-started/create-new"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: ServerIcon,
          description: `Incrementally add Turborepo to your existing monorepo codebase.`,
          name: "Add to existing monorepo",
        }}
        href="/docs/getting-started/existing-monorepo"
      ></DetailedFeatureLink>
    </div>
  );
};

export const LearnMoreArea = () => {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      <DetailedFeatureLink
        feature={{
          Icon: LightningBoltIcon,
          description: `The way you run your tasks is probably not optimized. Turborepo speeds them up with smart scheduling, minimising idle CPU's.`,
          name: "Maximum Multitasking",
        }}
        href="/docs/core-concepts/running-tasks"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: CloudUploadIcon,
          description: `Turborepo remembers the output of any task you run - and can skip work that's already been done.`,
          name: "Never do the same work twice",
        }}
        href="/docs/core-concepts/caching"
      ></DetailedFeatureLink>
    </div>
  );
};
