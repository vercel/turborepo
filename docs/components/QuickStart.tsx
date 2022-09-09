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

// const CloudUploadIcon = () => {
//   return (
//     <svg
//       xmlns="http://www.w3.org/2000/svg"
//       fill="none"
//       viewBox="0 0 24 24"
//       strokeWidth={1.5}
//       className="w-6 h-6"
//     >
//       <path
//         strokeLinecap="round"
//         strokeLinejoin="round"
//         stroke="url(#gradient)"
//         d="M12 16.5V9.75m0 0l3 3m-3-3l-3 3M6.75 19.5a4.5 4.5 0 01-1.41-8.775 5.25 5.25 0 0110.233-2.33 3 3 0 013.758 3.848A3.752 3.752 0 0118 19.5H6.75z"
//       />
//     </svg>
//   );
// };

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
