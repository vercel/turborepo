import type Image from "next/image";
import EcosystemIconDark from "../public/images/docs/shared/feature-icons/ecosystem-dark.svg";
import EcosystemIconLight from "../public/images/docs/shared/feature-icons/ecosystem-light.svg";
import LightningIconDark from "../public/images/docs/shared/feature-icons/lightning-dark.svg";
import LightningIconLight from "../public/images/docs/shared/feature-icons/lightning-light.svg";
import BarsIconDark from "../public/images/docs/shared/feature-icons/bars-dark.svg";
import BarsIconLight from "../public/images/docs/shared/feature-icons/bars-light.svg";
import MultiEnvTargetsIconDark from "../public/images/docs/shared/feature-icons/multi-env-targets-dark.svg";
import MultiEnvTargetsIconLight from "../public/images/docs/shared/feature-icons/multi-env-targets-light.svg";
import NextJSIconDark from "../public/images/docs/shared/feature-icons/nextjs-dark.svg";
import NextJSIconLight from "../public/images/docs/shared/feature-icons/nextjs-light.svg";
import ServerComponentsIconDark from "../public/images/docs/shared/feature-icons/server-components-dark.svg";
import ServerComponentsIconLight from "../public/images/docs/shared/feature-icons/server-components-light.svg";
import FingerprintIconDark from "../public/images/docs/shared/feature-icons/fingerprint-dark.svg";
import FingerprintIconLight from "../public/images/docs/shared/feature-icons/fingerprint-light.svg";
import CloudIconDark from "../public/images/docs/shared/feature-icons/cloud-dark.svg";
import CloudIconLight from "../public/images/docs/shared/feature-icons/cloud-light.svg";
import CpuIconDark from "../public/images/docs/shared/feature-icons/cpu-dark.svg";
import CpuIconLight from "../public/images/docs/shared/feature-icons/cpu-light.svg";
import PieconDark from "../public/images/docs/shared/feature-icons/piecon-dark.svg";
import PieconLight from "../public/images/docs/shared/feature-icons/piecon-light.svg";
import RefreshIconDark from "../public/images/docs/shared/feature-icons/refresh-dark.svg";
import RefreshIconLight from "../public/images/docs/shared/feature-icons/refresh-light.svg";
import ArrowsExpandIconDark from "../public/images/docs/shared/feature-icons/arrows-expand-dark.svg";
import ArrowsExpandIconLight from "../public/images/docs/shared/feature-icons/arrows-expand-light.svg";
import BeakerIconDark from "../public/images/docs/shared/feature-icons/beaker-dark.svg";
import BeakerIconLight from "../public/images/docs/shared/feature-icons/beaker-light.svg";

type NextImageSrc = Parameters<typeof Image>[0]["src"];

export type Feature = {
  name: string;
  description: string;
  iconDark: NextImageSrc;
  iconLight: NextImageSrc;
  page: "all" | "home" | "docs";
};

export type Features = Array<Feature>;

const REPO_FEATURES: Features = [
  {
    name: "Incremental builds",
    description: `Building once is painful enough, Turborepo will remember what you've built and skip the stuff that's already been computed.`,
    iconDark: RefreshIconDark,
    iconLight: RefreshIconLight,
    page: "all",
  },
  {
    name: "Content-aware hashing",
    description: `Turborepo looks at the contents of your files, not timestamps to figure out what needs to be built.`,
    iconDark: FingerprintIconDark,
    iconLight: FingerprintIconLight,
    page: "home",
  },
  {
    name: "Parallel execution",
    description: `Execute builds using every core at maximum parallelism without wasting idle CPUs.`,
    iconDark: LightningIconDark,
    iconLight: LightningIconLight,
    page: "all",
  },
  {
    name: "Remote Caching",
    description: `Share a remote build cache with your teammates and CI/CD for even faster builds.`,
    iconDark: CloudIconDark,
    iconLight: CloudIconLight,
    page: "all",
  },
  {
    name: "Zero runtime overhead",
    description: `Turborepo won’t interfere with your runtime code or touch your sourcemaps. `,
    iconDark: CpuIconDark,
    iconLight: CpuIconLight,
    page: "all",
  },
  {
    name: "Pruned subsets",
    description: `Speed up PaaS deploys by generating a subset of your monorepo with only what's needed to build a specific target.`,
    iconDark: PieconDark,
    iconLight: PieconLight,
    page: "all",
  },
  {
    name: "Task pipelines",
    description: `Define the relationships between your tasks and then let Turborepo optimize what to build and when.`,
    iconDark: ArrowsExpandIconDark,
    iconLight: ArrowsExpandIconLight,
    page: "all",
  },
  {
    name: "Meets you where you’re at",
    description: `Using Lerna? Keep your package publishing workflow and use Turborepo to turbocharge task running.`,
    iconDark: BeakerIconDark,
    iconLight: BeakerIconLight,
    page: "home",
  },
  {
    name: `Profile in your browser`,
    description: `Generate build profiles and import them in Chrome or Edge to understand which tasks are taking the longest.`,
    iconDark: BarsIconDark,
    iconLight: BarsIconLight,
    page: "home",
  },
];

const PACK_FEATURES: Features = [
  {
    name: "Incremental by design",
    description: `Building once is enough work—once Turbopack performs a task, it never does it again. `,
    iconDark: BarsIconDark,
    iconLight: BarsIconLight,
    page: "all",
  },
  {
    name: "Ecosystem-friendly",
    description: `Get out-of-the-box support for TypeScript, JSX, CSS, CSS Modules, WebAssembly, and more.`,
    iconDark: EcosystemIconDark,
    iconLight: EcosystemIconLight,
    page: "home",
  },
  {
    name: "Lightning fast HMR",
    description: `Hot Module Replacement (HMR) stays fast regardless of the size of your app.`,
    iconDark: LightningIconDark,
    iconLight: LightningIconLight,
    page: "all",
  },
  {
    name: "React Server Components",
    description: `Get native support for React Server Components when using Turbopack. `,
    iconDark: ServerComponentsIconDark,
    iconLight: ServerComponentsIconLight,
    page: "all",
  },
  {
    name: "Simultaneous Multiple Env Targets",
    description: `Build and optimize for multiple environments together (Browser, Server, Edge, SSR, React Server Components).`,
    iconDark: MultiEnvTargetsIconDark,
    iconLight: MultiEnvTargetsIconLight,
    page: "all",
  },
  {
    name: "Next.js support",
    description: `Turbopack will also power Next.js production builds, both locally and in the cloud.`,
    iconDark: NextJSIconDark,
    iconLight: NextJSIconLight,
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
