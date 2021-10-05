import {
  ArrowsExpandIcon,
  BeakerIcon,
  ChartBarIcon,
  ChartPieIcon,
  ChipIcon,
  CloudUploadIcon,
  FingerPrintIcon,
  LightningBoltIcon,
  RefreshIcon,
} from "@heroicons/react/outline";
import React from "react";

const features = [
  {
    name: "Incremental builds",
    description: `Building once is painful enough, Turborepo will remember what you've built and skip the stuff that's already been computed.`,
    icon: RefreshIcon,
  },
  {
    name: "Content-aware hashing",
    description: `Turborepo looks at the contents of your files, not timestamps to figure out what needs to be built.`,
    icon: FingerPrintIcon,
  },
  {
    name: "Cloud caching",
    description: `Share a cloud build cache with your teammates and CI/CD for even faster builds.`,
    icon: CloudUploadIcon,
  },
  {
    name: "Parallel execution",
    description: `Execute builds using every core at maximum parallelism without wasting idle CPUs.`,
    icon: LightningBoltIcon,
  },
  {
    name: "Task pipelines",
    description: `Define the relationships between your tasks and then let Turborepo optimize what to build and when.`,
    icon: ArrowsExpandIcon,
  },
  {
    name: "Zero runtime overhead",
    description: `Turborepo doesn't interfere with your runtime code or touch your sourcemaps. It does what it does and then gets out of your way.`,
    icon: ChipIcon,
  },
  // {
  //   name: 'Package manager agnostic',
  //   description: `Turborepo works with Yarn v1, Yarn v2, NPM, and PNPM workspaces.`,
  //   icon: LightningBoltIcon,
  // },
  // {
  //   name: 'Focused installs',
  //   description: `Only install the dependencies you actually need. Works perfectly with Docker layer caching.`,
  //   icon: DownloadIcon,
  // },
  {
    name: "Pruned subsets",
    description: `Speed up PaaS deploys by generating a subset of your monorepo with only what's needed to build a specific target.`,
    icon: ChartPieIcon,
  },
  {
    name: "JSON configuration",
    description: `Reduce complexity through convention. Fan out configuration with just a few lines of JSON.`,
    icon: BeakerIcon,
  },
  {
    name: `Profile in your browser`,
    description: `Generate build profiles and import them in Chrome or Edge to understand which tasks are taking the longest.`,
    icon: ChartBarIcon,
  },
];

function Features() {
  return (
    <>
      <div className="mt-4 text-2xl text-gray-600 dark:text-gray-500 font-medium">
        A blazing fast build system for JavaScript/TypeScript monorepos
      </div>
      <div className="my-12 grid grid-cols-2 gap-6 sm:grid-cols-3 ">
        {features.map((feature) => (
          <div className="flex items-center space-x-4" key={feature.name}>
            <div>
              <feature.icon className="h-6 w-6 " aria-hidden="true" />
            </div>
            <div>
              <div className="my-0 font-medium dark:text-white">
                {feature.name}
              </div>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

export default Features;
