import {
  BookmarkIcon,
  BriefcaseIcon,
  ChatAlt2Icon,
  CloudDownloadIcon,
  CloudUploadIcon,
  CodeIcon,
  LibraryIcon,
  PencilAltIcon,
  ShareIcon,
  ShieldExclamationIcon,
  StarIcon,
} from "@heroicons/react/outline";
import React from "react";
import { DetailedFeatureLink } from "./Feature";
import { GitHubIcon } from "./Icons";

const Wrapper = ({ children }: { children: React.ReactNode }) => {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      {children}
    </div>
  );
};

export const FundamentalsArea = () => {
  return (
    <Wrapper>
      <DetailedFeatureLink
        feature={{
          Icon: CloudDownloadIcon,
          description: `Learn how to install and manage packages in your monorepo.`,
          name: "Package Installation",
        }}
        href="/docs/handbook/package-installation"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: ChatAlt2Icon,
          description:
            "Understand how workspaces help you develop packages locally.",
          name: "Workspaces",
        }}
        href="/docs/handbook/workspaces"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: LibraryIcon,
          description:
            "Step-by-step guide on migrating from a multi-repo to a monorepo.",
          name: "Migrating to a Monorepo",
        }}
        href="/docs/handbook/migrating-to-a-monorepo"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: ShareIcon,
          description:
            "Learn how to share code easily using either internal or external packages.",
          name: "Sharing Code",
        }}
        href="/docs/handbook/sharing-code/basics"
      ></DetailedFeatureLink>
    </Wrapper>
  );
};

export const TasksArea = () => {
  return (
    <Wrapper>
      <DetailedFeatureLink
        feature={{
          Icon: PencilAltIcon,
          description: `Learn how to set up your dev scripts using Turborepo.`,
          name: "Development Tasks",
        }}
        href="/docs/handbook/dev"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: CodeIcon,
          description:
            "Get framework-specific guides for building your apps with Turborepo.",
          name: "Building your App",
        }}
        href="/docs/handbook/building-your-app"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: ShieldExclamationIcon,
          description:
            "Learn how to share linting configs and co-ordinate tasks across your repo.",
          name: "Linting",
        }}
        href="/docs/handbook/linting/basics"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: StarIcon,
          description: "Configure your integration or end-to-end tests easily.",
          name: "Testing",
        }}
        href="/docs/handbook/testing"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: CloudUploadIcon,
          description:
            "Bundle, version and publish packages to npm from your monorepo.",
          name: "Publishing Packages",
        }}
        href="/docs/handbook/publishing-packages/basics"
      ></DetailedFeatureLink>
      {/* <DetailedFeatureLink
        feature={{
          Icon: BookmarkIcon,
          description:
            "Set up code generators to scaffold new apps and packages from the CLI.",
          name: "Code Generation",
        }}
        href="/docs/handbook/code-generators"
      ></DetailedFeatureLink> */}
    </Wrapper>
  );
};
