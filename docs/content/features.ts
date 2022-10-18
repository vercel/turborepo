import React, { ComponentProps } from "react";
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
import { IconType } from "../components/Icons";

import EcosystemIconDark from "../public/images/docs/pack/features/ecosystem-dark.svg";
import EcosystemIconLight from "../public/images/docs/pack/features/ecosystem-light.svg";
import HMRIconDark from "../public/images/docs/pack/features/hmr-dark.svg";
import HMRIconLight from "../public/images/docs/pack/features/hmr-light.svg";
import IncrementalIconDark from "../public/images/docs/pack/features/incremental-dark.svg";
import IncrementalIconLight from "../public/images/docs/pack/features/incremental-light.svg";
import MultiEnvTargetsIconDark from "../public/images/docs/pack/features/multi-env-targets-dark.svg";
import MultiEnvTargetsIconLight from "../public/images/docs/pack/features/multi-env-targets-light.svg";
import NextJSIconDark from "../public/images/docs/pack/features/nextjs-dark.svg";
import NextJSIconLight from "../public/images/docs/pack/features/nextjs-light.svg";
import ServerComponentsIconDark from "../public/images/docs/pack/features/server-components-dark.svg";
import ServerComponentsIconLight from "../public/images/docs/pack/features/server-components-light.svg";

export type Feature = {
  name: string;
  description: React.ReactNode;
  Icon: IconType;
  page: "all" | "home" | "docs";
};

export type Features = Array<Feature>;

const REPO_FEATURES: Features = [
  {
    name: "Incremental builds",
    description: `Building once is painful enough, Turborepo will remember what you've built and skip the stuff that's already been computed.`,
    Icon: RefreshIcon,
    page: "all",
  },
  {
    name: "Content-aware hashing",
    description: `Turborepo looks at the contents of your files, not timestamps to figure out what needs to be built.`,
    Icon: FingerPrintIcon,
    page: "home",
  },
  {
    name: "Parallel execution",
    description: `Execute builds using every core at maximum parallelism without wasting idle CPUs.`,
    Icon: LightningBoltIcon,
    page: "all",
  },
  {
    name: "Remote Caching",
    description: `Share a remote build cache with your teammates and CI/CD for even faster builds.`,
    Icon: CloudUploadIcon,
    page: "all",
  },
  {
    name: "Zero runtime overhead",
    description: `Turborepo won’t interfere with your runtime code or touch your sourcemaps. `,
    Icon: ChipIcon,
    page: "all",
  },
  {
    name: "Pruned subsets",
    description: `Speed up PaaS deploys by generating a subset of your monorepo with only what's needed to build a specific target.`,
    Icon: ChartPieIcon,
    page: "all",
  },
  {
    name: "Task pipelines",
    description: `Define the relationships between your tasks and then let Turborepo optimize what to build and when.`,
    Icon: ArrowsExpandIcon,
    page: "all",
  },
  {
    name: "Meets you where you’re at",
    description: `Using Lerna? Keep your package publishing workflow and use Turborepo to turbocharge task running.`,
    Icon: BeakerIcon,
    page: "home",
  },
  {
    name: `Profile in your browser`,
    description: `Generate build profiles and import them in Chrome or Edge to understand which tasks are taking the longest.`,
    Icon: ChartBarIcon,
    page: "home",
  },
];

export const PACK_FEATURES = [
  {
    name: "Incremental by design",
    description: `Building once is enough work—once Turbopack performs a task, it never does it again. `,
    IconDark: IncrementalIconDark,
    IconLight: IncrementalIconLight,
    page: "all",
  },
  {
    name: "Ecosystem-friendly",
    description: `Get out-of-the-box support for TypeScript, JSX, CSS, CSS Modules, WebAssembly, and more.`,
    IconDark: EcosystemIconDark,
    IconLight: EcosystemIconLight,
    page: "home",
  },
  {
    name: "Lightning fast HMR",
    description: `Hot Module Replacement (HMR) stays fast regardless of the size of your app.`,
    IconDark: HMRIconDark,
    IconLight: HMRIconLight,
    page: "all",
  },
  {
    name: "React Server Components",
    description: `Get native support for React Server Components when using Turbopack. `,
    IconDark: ServerComponentsIconDark,
    IconLight: ServerComponentsIconLight,
    page: "all",
  },
  {
    name: "Simulaneous Multiple Env Targets",
    description: `Build and optimize for multiple environments together (Browser, Server, Edge, SSR, React Server Components).`,
    IconDark: MultiEnvTargetsIconDark,
    IconLight: MultiEnvTargetsIconLight,
    page: "all",
  },
  {
    name: "Next.js support",
    description: `Turbopack will also power Next.js production builds, both locally and in the cloud.`,
    IconDark: NextJSIconDark,
    IconLight: NextJSIconLight,
    page: "all",
  },
];
export const REPO_DOCS_FEATURES = REPO_FEATURES.filter(
  (f) => f.page === "docs" || f.page === "all"
);

export const REPO_HOME_FEATURES = REPO_FEATURES.filter(
  (f) => f.page === "home" || f.page === "all"
);

export const PACK_HOME_FEATURES = PACK_FEATURES.filter(
  (f) => f.page === "home" || f.page === "all"
);

export default REPO_FEATURES;
