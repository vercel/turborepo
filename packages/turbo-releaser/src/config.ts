import type { Platform } from "./types";

export const supportedPlatforms: Array<Platform> = [
  { os: "darwin", arch: "x64" },
  { os: "darwin", arch: "arm64" },
  { os: "linux", arch: "x64" },
  { os: "linux", arch: "arm64" },
  { os: "windows", arch: "x64" },
  { os: "windows", arch: "arm64" }
];

export const releasePackages = [
  { name: "turbo", directory: "packages/turbo", tarball: "turbo" },
  {
    name: "create-turbo",
    directory: "packages/create-turbo",
    tarball: "create-turbo"
  },
  {
    name: "@turbo/codemod",
    directory: "packages/turbo-codemod",
    tarball: "turbo-codemod"
  },
  {
    name: "turbo-ignore",
    directory: "packages/turbo-ignore",
    tarball: "turbo-ignore"
  },
  {
    name: "@turbo/workspaces",
    directory: "packages/turbo-workspaces",
    tarball: "turbo-workspaces"
  },
  {
    name: "@turbo/gen",
    directory: "packages/turbo-gen",
    tarball: "turbo-gen"
  },
  {
    name: "eslint-plugin-turbo",
    directory: "packages/eslint-plugin-turbo",
    tarball: "eslint-plugin-turbo"
  },
  {
    name: "eslint-config-turbo",
    directory: "packages/eslint-config-turbo",
    tarball: "eslint-config-turbo"
  },
  {
    name: "@turbo/types",
    directory: "packages/turbo-types",
    tarball: "turbo-types"
  }
] as const;

export const releaseBuildFilters = releasePackages
  .filter(({ name }) => name !== "turbo")
  .map(({ name }) => name);
