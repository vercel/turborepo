export const REDIRECTS_FOR_V2_DOCS = [
  {
    source: "/repo/docs/installing",
    destination: "/repo/docs/getting-started/installation",
  },
  {
    source: "/repo/docs/getting-started/from-example",
    destination:
      "/repo/docs/getting-started/installation#start-with-an-example",
  },
  {
    source: "/repo/docs/getting-started/create-new",
    destination: "/repo/docs/crafting-your-repository#from-zero-to-turbo",
  },
  {
    source: "/repo/docs/getting-started/add-to-project",
    destination: "/repo/docs/guides/single-package-workspaces",
  },
  {
    source: "/repo/docs/getting-started/existing-monorepo",
    destination: "/repo/docs/getting-started/add-to-existing-repository",
  },
  {
    source: "/repo/docs/core-concepts/caching",
    destination: "/repo/docs/crafting-your-repository/caching",
  },
  {
    source: "/repo/docs/core-concepts/caching/to-cache-or-not-to-cache",
    destination:
      "/repo/docs/crafting-your-repository/caching#caching-a-task-is-slower-than-executing-the-task",
  },
  {
    source: "/repo/docs/core-concepts/caching/what-to-cache",
    destination: "/repo/docs/crafting-your-repository/caching#task-outputs",
  },
  {
    source: "/repo/docs/core-concepts/caching/file-inputs",
    destination: "/repo/docs/crafting-your-repository/caching#task-inputs",
  },
  {
    source: "/repo/docs/core-concepts/caching/environment-variable-inputs",
    destination:
      "/repo/docs/crafting-your-repository/using-environment-variables",
  },
  {
    source: "/repo/docs/core-concepts/monorepos",
    destination: "/repo/docs",
  },
  {
    source: "/repo/docs/core-concepts/monorepos/running-tasks",
    destination: "/repo/docs/crafting-your-repository/running-tasks",
  },
  {
    source: "/repo/docs/core-concepts/monorepos/task-dependencies",
    destination: "/repo/docs/crafting-your-repository/configuring-tasks",
  },
  {
    source: "/repo/docs/core-concepts/monorepos/task-graph",
    destination: "/repo/docs/core-concepts/package-and-task-graph#task-graph",
  },
  {
    source: "/repo/docs/core-concepts/monorepos/filtering",
    destination:
      "/repo/docs/crafting-your-repository/running-tasks#using-filters",
  },
  {
    source: "/repo/docs/core-concepts/monorepos/code-generation",
    destination: "/repo/docs/guides/generating-code",
  },
  {
    source: "/repo/docs/core-concepts/monorepos/skipping-tasks",
    destination: "/repo/docs/guides/skipping-tasks",
  },
  {
    source: "/repo/docs/core-concepts/monorepos/configuring-workspaces",
    destination: "/repo/docs/reference/package-configurations",
  },
  {
    source: "/repo/docs/reference/command-line-reference",
    destination: "/repo/docs/reference",
  },
  {
    source: "/repo/docs/reference/command-line-reference/:path",
    destination: "/repo/docs/reference/:path",
  },
  {
    source: "/repo/docs/reference/codemods",
    destination: "/repo/docs/reference/turbo-codemod",
  },
  {
    source: "/repo/docs/reference/system-variables",
    destination: "/repo/docs/reference/system-environment-variables",
  },
  {
    source: "/repo/docs/ci",
    destination: "/repo/docs/guides/ci-vendors",
  },
  {
    source: "/repo/docs/ci/:path",
    destination: "/repo/docs/guides/ci-vendors/:path",
  },
  {
    source: "/repo/docs/troubleshooting",
    destination: "/repo/docs/crafting-your-repository/caching#troubleshooting",
  },
  {
    source: "/repo/docs/handbook",
    destination: "/repo/docs/crafting-your-repository",
  },
  {
    source: "/repo/docs/handbook/what-is-a-monorepo",
    destination: "/repo/docs/crafting-your-repository/structuring-a-repository",
  },
  {
    source: "/repo/docs/handbook/package-installation",
    destination: "/repo/docs/crafting-your-repository/managing-dependencies",
  },
  {
    source: "/repo/docs/handbook/workspaces",
    destination: "/repo/docs/crafting-your-repository/structuring-a-repository",
  },
  {
    source: "/repo/docs/handbook/migrating-to-a-monorepo",
    destination: "/repo/docs/getting-started/add-to-existing-repository",
  },
  {
    source: "/repo/docs/handbook/dev",
    destination: "/repo/docs/crafting-your-repository/developing-applications",
  },
  {
    source: "/repo/docs/handbook/building-your-app",
    destination:
      "/repo/docs/crafting-your-repository/configuring-tasks#defining-tasks",
  },
  {
    source: "/repo/docs/handbook/deploying-with-docker",
    destination: "/repo/docs/guides/tools/docker",
  },
  {
    source: "/repo/docs/handbook/environment-variables",
    destination:
      "/repo/docs/crafting-your-repository/using-environment-variables",
  },
  {
    source: "/repo/docs/handbook/sharing-code",
    destination: "/repo/docs/core-concepts/internal-packages",
  },
  {
    source: "/repo/docs/handbook/sharing-code/internal-packages",
    destination: "/repo/docs/core-concepts/internal-packages",
  },
  {
    source: "/repo/docs/handbook/linting",
    destination: "/repo/docs/guides/tools",
  },
  {
    source: "/repo/docs/handbook/linting/typescript",
    destination: "/repo/docs/guides/tools/typescript",
  },
  {
    source: "/repo/docs/handbook/linting/eslint",
    destination: "/repo/docs/guides/tools/eslint",
  },
  {
    source: "/repo/docs/handbook/testing",
    destination: "/repo/docs/guides/tools",
  },
  {
    source: "/repo/docs/handbook/publishing-packages",
    destination: "/repo/docs/guides/publishing-packages",
  },
  {
    source: "/repo/docs/handbook/publishing-packages/bundling",
    destination: "/repo/docs/guides/publishing-packages",
  },
  {
    source: "/repo/docs/handbook/publishing-packages/versioning-and-publishing",
    destination: "/repo/docs/guides/publishing-packages",
  },
  {
    source: "/repo/docs/handbook/troubleshooting",
    destination:
      "/repo/docs/crafting-your-repository/managing-dependencies#keeping-dependencies-on-the-same-version",
  },
  {
    source: "/repo/docs/handbook/tools/prisma",
    destination: "/repo/docs/guides/tools/prisma",
  },
  {
    source: "/repo/docs/handbook/tools/storybook",
    destination: "/repo/docs/guides/tools/storybook",
  },
  {
    source: "/repo/docs/support",
    destination: "/repo/docs/getting-started/support-policy",
  },
  {
    source: "/repo/docs/acknowledgements",
    destination: "/repo/docs/community#acknowledgements",
  },
  {
    source: "/repo/docs/faq",
    destination: "/repo/docs",
  },
  {
    source: "/repo/docs/guides/publishing-packages",
    destination: "/repo/docs/guides/publishing-libraries",
  },
  {
    source: "/docs/features/filtering",
    destination:
      "/repo/docs/crafting-your-repository/running-tasks#using-filters",
  },
  {
    source: "/messages/:slug",
    destination: "/repo/docs/messages/:slug",
  },
  // Used by create-turbo >=2.0.5
  {
    source: "/repo/remote-cache",
    destination: "/repo/docs/core-concepts/remote-caching",
  },
  {
    source: "/repo/docs/upgrading-to-v1",
    destination: "/repo/docs/crafting-your-repository/upgrading",
  },
  {
    source: "/repo",
    destination: "/repo/docs",
  },
  {
    source: "/pack",
    destination: "/pack/docs",
  },
  {
    source: "/privacy",
    destination: "https://vercel.com/legal/privacy-policy",
  },
  {
    source: "/repo/docs/reference/options-one-pager",
    destination: "/repo/docs/reference/options-overview",
  },
];
