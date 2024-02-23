// This data is used for local development
// since the index is generated at build time.

export const mockSearchData = [
  {
    id: "en_2c55a7b",
    score: 6.299574,
    words: [1, 9, 45, 51, 70, 192, 217, 242, 260, 268, 288, 294, 313, 468, 483],
    data: {
      url: "/_next/static/chunks/server/pages/repo/docs/reference/command-line-reference/prune.html",
      content:
        "turbo prune <scope>... Generate a sparse/partial monorepo with a pruned lockfile for a target workspace. This command will generate folder called out with the following inside of it: The full source code of all internal workspaces that are needed to build the target. A new pruned lockfile that only contains the pruned subset of the original root lockfile with the dependencies that are actually used by the workspaces in the pruned workspace. A copy of the root package.json. . # Folder full source code for all workspaces needed to build the target ├── package.json # The root `package.json` ├── packages │ ├── ui │ │ ├── package.json │ │ ├── src │ │ │ └── index.tsx │ │ └── tsconfig.json │ ├── shared │ │ ├── package.json │ │ ├── src │ │ │ ├── __tests__ │ │ │ │ ├── sum.test.ts │ │ │ │ └── tsconfig.json │ │ │ ├── index.ts │ │ │ └── sum.ts │ │ └── tsconfig.json │ └── frontend │ ├── next-env.d.ts │ ├── next.config.js │ ├── package.json │ ├── src │ │ └── pages │ │ └── index.tsx │ └── tsconfig.json └── yarn.lock # The pruned lockfile for all targets in the subworkspace Options --docker type: boolean. Default to false. Passing this flag will alter the outputted folder with the pruned workspace to make it easier to use with Docker best practices / layer caching (opens in a new tab). With the --docker flag. The prune command will generate folder called out with the following inside of it: A folder json with the pruned workspace's package.jsons. A folder full with the pruned workspace's full source code, but only including the internal packages that are needed to build the target. A new pruned lockfile that only contains the pruned subset of the original root lockfile with the dependencies that are actually used by the packages in the pruned workspace. . ├── full # Folder full source code for all package needed to build the target │ ├── package.json │ └── packages │ ├── ui │ │ ├── package.json │ │ ├── src │ │ │ └── index.tsx │ │ └── tsconfig.json │ ├── shared │ │ ├── package.json │ │ ├── src │ │ │ ├── __tests__ │ │ │ │ ├── sum.test.ts │ │ │ │ └── tsconfig.json │ │ │ ├── index.ts │ │ │ └── sum.ts │ │ └── tsconfig.json │ └── frontend │ ├── next-env.d.ts │ ├── next.config.js │ ├── package.json │ ├── src │ │ └── pages │ │ └── index.tsx │ └── tsconfig.json ├── json # Folder containing just package.jsons for all targets in the subworkspace │ ├── package.json │ └── packages │ ├── ui │ │ └── package.json │ ├── shared │ │ └── package.json │ └── frontend │ └── package.json └── yarn.lock # The pruned lockfile for all targets in the subworkspace --out-dir Default: ./out. Customize the directory the pruned output is generated in.",
      word_count: 488,
      filters: {},
      meta: {
        title: "turbo prune <scope>...",
      },
      anchors: [
        {
          element: "a",
          id: "options",
          text: "",
          location: 201,
        },
        {
          element: "a",
          id: "--docker",
          text: "",
          location: 202,
        },
        {
          element: "a",
          id: "--out-dir",
          text: "",
          location: 477,
        },
      ],
      weighted_locations: [
        {
          weight: 7,
          balanced_score: 58784.535,
          location: 1,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 9,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 45,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 51,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 70,
        },
        {
          weight: 0.5,
          balanced_score: 299.92108,
          location: 192,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 217,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 242,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 260,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 268,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 288,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 294,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 313,
        },
        {
          weight: 0.5,
          balanced_score: 299.92108,
          location: 468,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 483,
        },
      ],
      locations: [
        1, 9, 45, 51, 70, 192, 217, 242, 260, 268, 288, 294, 313, 468, 483,
      ],
      raw_content:
        "turbo prune &lt;scope&gt;... Generate a sparse/partial monorepo with a pruned lockfile for a target workspace. This command will generate folder called out with the following inside of it: The full source code of all internal workspaces that are needed to build the target. A new pruned lockfile that only contains the pruned subset of the original root lockfile with the dependencies that are actually used by the workspaces in the pruned workspace. A copy of the root package.json. . # Folder full source code for all workspaces needed to build the target ├── package.json # The root `package.json` ├── packages │ ├── ui │ │ ├── package.json │ │ ├── src │ │ │ └── index.tsx │ │ └── tsconfig.json │ ├── shared │ │ ├── package.json │ │ ├── src │ │ │ ├── __tests__ │ │ │ │ ├── sum.test.ts │ │ │ │ └── tsconfig.json │ │ │ ├── index.ts │ │ │ └── sum.ts │ │ └── tsconfig.json │ └── frontend │ ├── next-env.d.ts │ ├── next.config.js │ ├── package.json │ ├── src │ │ └── pages │ │ └── index.tsx │ └── tsconfig.json └── yarn.lock # The pruned lockfile for all targets in the subworkspace Options --docker type: boolean. Default to false. Passing this flag will alter the outputted folder with the pruned workspace to make it easier to use with Docker best practices / layer caching (opens in a new tab). With the --docker flag. The prune command will generate folder called out with the following inside of it: A folder json with the pruned workspace's package.jsons. A folder full with the pruned workspace's full source code, but only including the internal packages that are needed to build the target. A new pruned lockfile that only contains the pruned subset of the original root lockfile with the dependencies that are actually used by the packages in the pruned workspace. . ├── full # Folder full source code for all package needed to build the target │ ├── package.json │ └── packages │ ├── ui │ │ ├── package.json │ │ ├── src │ │ │ └── index.tsx │ │ └── tsconfig.json │ ├── shared │ │ ├── package.json │ │ ├── src │ │ │ ├── __tests__ │ │ │ │ ├── sum.test.ts │ │ │ │ └── tsconfig.json │ │ │ ├── index.ts │ │ │ └── sum.ts │ │ └── tsconfig.json │ └── frontend │ ├── next-env.d.ts │ ├── next.config.js │ ├── package.json │ ├── src │ │ └── pages │ │ └── index.tsx │ └── tsconfig.json ├── json # Folder containing just package.jsons for all targets in the subworkspace │ ├── package.json │ └── packages │ ├── ui │ │ └── package.json │ ├── shared │ │ └── package.json │ └── frontend │ └── package.json └── yarn.lock # The pruned lockfile for all targets in the subworkspace --out-dir Default: ./out. Customize the directory the pruned output is generated in.",
      raw_url:
        "/server/pages/repo/docs/reference/command-line-reference/prune.html",
      excerpt:
        "turbo <mark>prune</mark> &lt;scope&gt;... Generate a sparse/partial monorepo with a <mark>pruned</mark> lockfile for a target workspace. This command will generate folder called out with the following inside of it: The full",
      sub_results: [
        {
          title: "turbo prune <scope>...",
          url: "/_next/static/chunks/server/pages/repo/docs/reference/command-line-reference/prune.html",
          weighted_locations: [
            {
              weight: 7,
              balanced_score: 58784.535,
              location: 1,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 9,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 45,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 51,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 70,
            },
            {
              weight: 0.5,
              balanced_score: 299.92108,
              location: 192,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 217,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 242,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 260,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 268,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 288,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 294,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 313,
            },
            {
              weight: 0.5,
              balanced_score: 299.92108,
              location: 468,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 483,
            },
          ],
          locations: [
            1, 9, 45, 51, 70, 192, 217, 242, 260, 268, 288, 294, 313, 468, 483,
          ],
          excerpt:
            "turbo <mark>prune</mark> &lt;scope&gt;... Generate a sparse/partial monorepo with a <mark>pruned</mark> lockfile for a target workspace. This command will generate folder called out with the following inside of it: The full",
        },
      ],
    },
  },
  {
    id: "en_aa8416b",
    score: 0.42313924,
    words: [328, 347, 349, 357, 377, 395, 524, 547, 616],
    data: {
      url: "/_next/static/chunks/server/pages/repo/docs/handbook/deploying-with-docker.html",
      content:
        "Deploying with Docker. Building a Docker (opens in a new tab) image is a common way to deploy all sorts of applications. However, doing so from a monorepo has several challenges. The problem TL;DR: In a monorepo, unrelated changes can make Docker do unnecessary work when deploying your app. Let's imagine you have a monorepo that looks like this: ├── apps │ ├── docs │ │ ├── server.js │ │ └── package.json │ └── web │ └── package.json ├── package.json └── package-lock.json You want to deploy apps/docs using Docker, so you create a Dockerfile: FROM node:16 WORKDIR /usr/src/app # Copy root package.json and lockfile COPY package.json ./ COPY package-lock.json ./ # Copy the docs package.json COPY apps/docs/package.json ./apps/docs/package.json RUN npm install # Copy app source COPY . . EXPOSE 8080 CMD [ \"node\", \"apps/docs/server.js\" ] This will copy the root package.json and the root lockfile to the docker image. Then, it'll install dependencies, copy the app source and start the app. You should also create a .dockerignore file to prevent node_modules from being copied in with the app's source. node_modules npm-debug.log The lockfile changes too often Docker is pretty smart about how it deploys your apps. Just like Turbo, it tries to do as little work as possible (opens in a new tab). In our Dockerfile's case, it will only run npm install if the files it has in its image are different from the previous run. If not, it'll restore the node_modules directory it had before. This means that whenever package.json, apps/docs/package.json or package-lock.json change, the docker image will run npm install. This sounds great - until we realise something. The package-lock.json is global for the monorepo. That means that if we install a new package inside apps/web, we'll cause apps/docs to redeploy. In a large monorepo, this can result in a huge amount of lost time, as any change to a monorepo's lockfile cascades into tens or hundreds of deploys. The solution The solution is to prune the inputs to the Dockerfile to only what is strictly necessary. Turborepo provides a simple solution - turbo prune. turbo prune docs --docker Running this command creates a pruned version of your monorepo inside an ./out directory. It only includes workspaces which docs depends on. Crucially, it also prunes the lockfile so that only the relevant node_modules will be downloaded. The --docker flag By default, turbo prune puts all relevant files inside ./out. But to optimize caching with Docker, we ideally want to copy the files over in two stages. First, we want to copy over only what we need to install the packages. When running --docker, you'll find this inside ./out/json. out ├── json │ ├── apps │ │ └── docs │ │ └── package.json │ └── package.json ├── full │ ├── apps │ │ └── docs │ │ ├── server.js │ │ └── package.json │ ├── package.json │ └── turbo.json └── package-lock.json Afterwards, you can copy the files in ./out/full to add the source files. Splitting up dependencies and source files in this way lets us only run npm install when dependencies change - giving us a much larger speedup. Without --docker, all pruned files are placed inside ./out. Example Our detailed with-docker example (opens in a new tab) goes into depth on how to utilise prune to its full potential. Here's the Dockerfile, copied over for convenience. This Dockerfile is written for a Next.js (opens in a new tab) app that is using the standalone output mode (opens in a new tab). FROM node:18-alpine AS base FROM base AS builder RUN apk add --no-cache libc6-compat RUN apk update # Set working directory WORKDIR /app RUN yarn global add turbo COPY . . RUN turbo prune web --docker # Add lockfile and package.json's of isolated subworkspace FROM base AS installer RUN apk add --no-cache libc6-compat RUN apk update WORKDIR /app # First install the dependencies (as they change less often) COPY .gitignore .gitignore COPY --from=builder /app/out/json/ . COPY --from=builder /app/out/yarn.lock ./yarn.lock RUN yarn install # Build the project COPY --from=builder /app/out/full/ . RUN yarn turbo run build --filter=web... FROM base AS runner WORKDIR /app # Don't run production as root RUN addgroup --system --gid 1001 nodejs RUN adduser --system --uid 1001 nextjs USER nextjs COPY --from=installer /app/apps/web/next.config.js . COPY --from=installer /app/apps/web/package.json . # Automatically leverage output traces to reduce image size # https://nextjs.org/docs/advanced-features/output-file-tracing COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/standalone ./ COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/static ./apps/web/.next/static COPY --from=installer --chown=nextjs:nodejs /app/apps/web/public ./apps/web/public CMD node apps/web/server.js Remote caching To take advantage of remote caches during Docker builds, you will need to make sure your build container has credentials to access your Remote Cache. There are many ways to take care of secrets in a Docker image. We will use a simple strategy here with multi-stage builds using secrets as build arguments that will get hidden for the final image. Assuming you are using a Dockerfile similar to the one above, we will bring in some environment variables from build arguments right before turbo build: ARG TURBO_TEAM ENV TURBO_TEAM=$TURBO_TEAM ARG TURBO_TOKEN ENV TURBO_TOKEN=$TURBO_TOKEN RUN yarn turbo run build --filter=web... turbo will now be able to hit your remote cache. To see a Turborepo cache hit for a non-cached Docker build image, run a command like this one from your project root: docker build -f apps/web/Dockerfile . --build-arg TURBO_TEAM=“your-team-name” --build-arg TURBO_TOKEN=“your-token“ --no-cache",
      word_count: 886,
      filters: {},
      meta: {
        title: "Deploying with Docker",
      },
      anchors: [
        {
          element: "a",
          id: "the-problem",
          text: "",
          location: 33,
        },
        {
          element: "a",
          id: "the-lockfile-changes-too-often",
          text: "",
          location: 186,
        },
        {
          element: "a",
          id: "the-solution",
          text: "",
          location: 324,
        },
        {
          element: "a",
          id: "the---docker-flag",
          text: "",
          location: 392,
        },
        {
          element: "a",
          id: "example",
          text: "",
          location: 531,
        },
        {
          element: "a",
          id: "remote-caching",
          text: "",
          location: 744,
        },
      ],
      weighted_locations: [
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 328,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 347,
        },
        {
          weight: 0.5,
          balanced_score: 299.92108,
          location: 349,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 357,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 377,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 395,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 524,
        },
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 547,
        },
        {
          weight: 0.5,
          balanced_score: 299.92108,
          location: 616,
        },
      ],
      locations: [328, 347, 349, 357, 377, 395, 524, 547, 616],
      raw_content:
        "Deploying with Docker. Building a Docker (opens in a new tab) image is a common way to deploy all sorts of applications. However, doing so from a monorepo has several challenges. The problem TL;DR: In a monorepo, unrelated changes can make Docker do unnecessary work when deploying your app. Let's imagine you have a monorepo that looks like this: ├── apps │ ├── docs │ │ ├── server.js │ │ └── package.json │ └── web │ └── package.json ├── package.json └── package-lock.json You want to deploy apps/docs using Docker, so you create a Dockerfile: FROM node:16 WORKDIR /usr/src/app # Copy root package.json and lockfile COPY package.json ./ COPY package-lock.json ./ # Copy the docs package.json COPY apps/docs/package.json ./apps/docs/package.json RUN npm install # Copy app source COPY . . EXPOSE 8080 CMD [ \"node\", \"apps/docs/server.js\" ] This will copy the root package.json and the root lockfile to the docker image. Then, it'll install dependencies, copy the app source and start the app. You should also create a .dockerignore file to prevent node_modules from being copied in with the app's source. node_modules npm-debug.log The lockfile changes too often Docker is pretty smart about how it deploys your apps. Just like Turbo, it tries to do as little work as possible (opens in a new tab). In our Dockerfile's case, it will only run npm install if the files it has in its image are different from the previous run. If not, it'll restore the node_modules directory it had before. This means that whenever package.json, apps/docs/package.json or package-lock.json change, the docker image will run npm install. This sounds great - until we realise something. The package-lock.json is global for the monorepo. That means that if we install a new package inside apps/web, we'll cause apps/docs to redeploy. In a large monorepo, this can result in a huge amount of lost time, as any change to a monorepo's lockfile cascades into tens or hundreds of deploys. The solution The solution is to prune the inputs to the Dockerfile to only what is strictly necessary. Turborepo provides a simple solution - turbo prune. turbo prune docs --docker Running this command creates a pruned version of your monorepo inside an ./out directory. It only includes workspaces which docs depends on. Crucially, it also prunes the lockfile so that only the relevant node_modules will be downloaded. The --docker flag By default, turbo prune puts all relevant files inside ./out. But to optimize caching with Docker, we ideally want to copy the files over in two stages. First, we want to copy over only what we need to install the packages. When running --docker, you'll find this inside ./out/json. out ├── json │ ├── apps │ │ └── docs │ │ └── package.json │ └── package.json ├── full │ ├── apps │ │ └── docs │ │ ├── server.js │ │ └── package.json │ ├── package.json │ └── turbo.json └── package-lock.json Afterwards, you can copy the files in ./out/full to add the source files. Splitting up dependencies and source files in this way lets us only run npm install when dependencies change - giving us a much larger speedup. Without --docker, all pruned files are placed inside ./out. Example Our detailed with-docker example (opens in a new tab) goes into depth on how to utilise prune to its full potential. Here's the Dockerfile, copied over for convenience. This Dockerfile is written for a Next.js (opens in a new tab) app that is using the standalone output mode (opens in a new tab). FROM node:18-alpine AS base FROM base AS builder RUN apk add --no-cache libc6-compat RUN apk update # Set working directory WORKDIR /app RUN yarn global add turbo COPY . . RUN turbo prune web --docker # Add lockfile and package.json's of isolated subworkspace FROM base AS installer RUN apk add --no-cache libc6-compat RUN apk update WORKDIR /app # First install the dependencies (as they change less often) COPY .gitignore .gitignore COPY --from=builder /app/out/json/ . COPY --from=builder /app/out/yarn.lock ./yarn.lock RUN yarn install # Build the project COPY --from=builder /app/out/full/ . RUN yarn turbo run build --filter=web... FROM base AS runner WORKDIR /app # Don't run production as root RUN addgroup --system --gid 1001 nodejs RUN adduser --system --uid 1001 nextjs USER nextjs COPY --from=installer /app/apps/web/next.config.js . COPY --from=installer /app/apps/web/package.json . # Automatically leverage output traces to reduce image size # https://nextjs.org/docs/advanced-features/output-file-tracing COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/standalone ./ COPY --from=installer --chown=nextjs:nodejs /app/apps/web/.next/static ./apps/web/.next/static COPY --from=installer --chown=nextjs:nodejs /app/apps/web/public ./apps/web/public CMD node apps/web/server.js Remote caching To take advantage of remote caches during Docker builds, you will need to make sure your build container has credentials to access your Remote Cache. There are many ways to take care of secrets in a Docker image. We will use a simple strategy here with multi-stage builds using secrets as build arguments that will get hidden for the final image. Assuming you are using a Dockerfile similar to the one above, we will bring in some environment variables from build arguments right before turbo build: ARG TURBO_TEAM ENV TURBO_TEAM=$TURBO_TEAM ARG TURBO_TOKEN ENV TURBO_TOKEN=$TURBO_TOKEN RUN yarn turbo run build --filter=web... turbo will now be able to hit your remote cache. To see a Turborepo cache hit for a non-cached Docker build image, run a command like this one from your project root: docker build -f apps/web/Dockerfile . --build-arg TURBO_TEAM=“your-team-name” --build-arg TURBO_TOKEN=“your-token“ --no-cache",
      raw_url: "/server/pages/repo/docs/handbook/deploying-with-docker.html",
      excerpt:
        "to <mark>prune</mark> the inputs to the Dockerfile to only what is strictly necessary. Turborepo provides a simple solution - turbo <mark>prune.</mark> turbo <mark>prune</mark> docs --docker Running this command creates a",
      sub_results: [
        {
          title: "Deploying with Docker",
          url: "/_next/static/chunks/server/pages/repo/docs/handbook/deploying-with-docker.html",
          weighted_locations: [
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 328,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 347,
            },
            {
              weight: 0.5,
              balanced_score: 299.92108,
              location: 349,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 357,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 377,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 395,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 524,
            },
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 547,
            },
            {
              weight: 0.5,
              balanced_score: 299.92108,
              location: 616,
            },
          ],
          locations: [328, 347, 349, 357, 377, 395, 524, 547, 616],
          excerpt:
            "to <mark>prune</mark> the inputs to the Dockerfile to only what is strictly necessary. Turborepo provides a simple solution - turbo <mark>prune.</mark> turbo <mark>prune</mark> docs --docker Running this command creates a",
        },
      ],
    },
  },
  {
    id: "en_880cdc0",
    score: 0.24032138,
    words: [188],
    data: {
      url: "/_next/static/chunks/server/pages/repo/docs/handbook.html",
      content:
        "Monorepo Handbook. Now we've covered the core concepts, it's time to get practical. This handbook covers everything you need to know to set up and use your monorepo. Fundamentals Learn about the fundamental building blocks of monorepos - workspaces, packages and dependencies. What is a Monorepo? Understand how a monorepo compares to a polyrepo, and what problems it solves. Package Installation. Learn how to install and manage packages in your monorepo. Workspaces. Understand how workspaces help you develop packages locally. Migrating to a Monorepo. Step-by-step guide on migrating from a multi-repo to a monorepo. Sharing Code. Learn how to share code easily using either internal or external packages. Troubleshooting. Learn the common monorepo pain points, and how to fix them. Tasks Configure common tasks in your monorepo, like linting, testing, and building your apps and packages. Development Tasks. Learn how to set up your dev scripts using Turborepo. Building your App. Get framework-specific guides for building your apps with Turborepo. Linting. Learn how to share linting configs and co-ordinate tasks across your repo. Testing. Configure your integration or end-to-end tests easily. Deploying with Docker. Make use of Turborepo's prune command to keep your Docker deploys fast. Publishing Packages. Bundle, version and publish packages to npm from your monorepo.",
      word_count: 208,
      filters: {},
      meta: {
        title: "Monorepo Handbook",
      },
      anchors: [
        {
          element: "a",
          id: "fundamentals",
          text: "",
          location: 29,
        },
        {
          element: "a",
          id: "tasks",
          text: "",
          location: 121,
        },
      ],
      weighted_locations: [
        {
          weight: 1,
          balanced_score: 1199.6843,
          location: 188,
        },
      ],
      locations: [188],
      raw_content:
        "Monorepo Handbook. Now we've covered the core concepts, it's time to get practical. This handbook covers everything you need to know to set up and use your monorepo. Fundamentals Learn about the fundamental building blocks of monorepos - workspaces, packages and dependencies. What is a Monorepo? Understand how a monorepo compares to a polyrepo, and what problems it solves. Package Installation. Learn how to install and manage packages in your monorepo. Workspaces. Understand how workspaces help you develop packages locally. Migrating to a Monorepo. Step-by-step guide on migrating from a multi-repo to a monorepo. Sharing Code. Learn how to share code easily using either internal or external packages. Troubleshooting. Learn the common monorepo pain points, and how to fix them. Tasks Configure common tasks in your monorepo, like linting, testing, and building your apps and packages. Development Tasks. Learn how to set up your dev scripts using Turborepo. Building your App. Get framework-specific guides for building your apps with Turborepo. Linting. Learn how to share linting configs and co-ordinate tasks across your repo. Testing. Configure your integration or end-to-end tests easily. Deploying with Docker. Make use of Turborepo's prune command to keep your Docker deploys fast. Publishing Packages. Bundle, version and publish packages to npm from your monorepo.",
      raw_url: "/server/pages/repo/docs/handbook.html",
      excerpt:
        "Testing. Configure your integration or end-to-end tests easily. Deploying with Docker. Make use of Turborepo's <mark>prune</mark> command to keep your Docker deploys fast. Publishing Packages. Bundle, version and publish packages",
      sub_results: [
        {
          title: "Monorepo Handbook",
          url: "/_next/static/chunks/server/pages/repo/docs/handbook.html",
          weighted_locations: [
            {
              weight: 1,
              balanced_score: 1199.6843,
              location: 188,
            },
          ],
          locations: [188],
          excerpt:
            "Testing. Configure your integration or end-to-end tests easily. Deploying with Docker. Make use of Turborepo's <mark>prune</mark> command to keep your Docker deploys fast. Publishing Packages. Bundle, version and publish packages",
        },
      ],
    },
  },
  // {
  //   id: "en_be7a354",
  //   score: 0.18934412,
  //   words: [131],
  //   data: {
  //     url: "/_next/static/chunks/server/pages/repo/docs/getting-started/from-example.html",
  //     content:
  //       "Turborepo Examples. Clone a Turborepo starter repository to get a head start on your monorepo. Next.js. Minimal Turborepo example for learning the fundamentals. examples/basic. Start BuildingDeploy Now. SvelteKit. Monorepo with multiple SvelteKit apps sharing a UI Library examples/with-svelte. Start BuildingDeploy Now. Design System. Unify your site's look and feel by sharing a design system across multiple apps. examples/design-system. Start BuildingDeploy Now. Gatsby.js. Monorepo with a Gatsby.js and a Next.js app both sharing a UI Library examples/with-gatsby. Start BuildingDeploy Now. Kitchen Sink. Want to see a more in-depth example? Includes multiple frameworks, both frontend and backend. examples/kitchen-sink. Start BuildingDeploy Now. React Native. Simple React Native & Next.js monorepo with a shared UI library examples/with-react-native-web. Start BuildingDeploy Now. Docker. Monorepo with an Express API and a Next.js App deployed with Docker utilizing turbo prune examples/with-docker. Start Building. Monorepo with Changesets. Simple Next.js monorepo preconfigured to publish packages via Changesets examples/with-changesets. Start Building. Non-Monorepo. Example of using Turborepo in a single project without workspaces examples/non-monorepo. Start Building. Prisma. Monorepo with a Next.js App fully configured with Prisma examples/with-prisma. Start Building. Rollup. Monorepo with a single Next.js app sharing a UI library bundled with Rollup examples/with-rollup. Start Building. Tailwind CSS. Monorepo with multiple Next.js apps sharing a UI Library all using Tailwind CSS with a shared config examples/with-tailwind. Start Building. Vite. Monorepo with multiple Vanilla JS apps bundled with Vite, sharing a UI Library examples/with-vite. Start Building. Vue/Nuxt. Monorepo with Vue and Nuxt, sharing a UI Library examples/with-vue-nuxt. Start Building. For even more examples and starters, see the Turborepo examples directory on GitHub (opens in a new tab).",
  //     word_count: 264,
  //     filters: {},
  //     meta: {
  //       title: "Turborepo Examples",
  //     },
  //     anchors: [],
  //     weighted_locations: [
  //       {
  //         weight: 1,
  //         balanced_score: 1199.6843,
  //         location: 131,
  //       },
  //     ],
  //     locations: [131],
  //     raw_content:
  //       "Turborepo Examples. Clone a Turborepo starter repository to get a head start on your monorepo. Next.js. Minimal Turborepo example for learning the fundamentals. examples/basic. Start BuildingDeploy Now. SvelteKit. Monorepo with multiple SvelteKit apps sharing a UI Library examples/with-svelte. Start BuildingDeploy Now. Design System. Unify your site's look and feel by sharing a design system across multiple apps. examples/design-system. Start BuildingDeploy Now. Gatsby.js. Monorepo with a Gatsby.js and a Next.js app both sharing a UI Library examples/with-gatsby. Start BuildingDeploy Now. Kitchen Sink. Want to see a more in-depth example? Includes multiple frameworks, both frontend and backend. examples/kitchen-sink. Start BuildingDeploy Now. React Native. Simple React Native & Next.js monorepo with a shared UI library examples/with-react-native-web. Start BuildingDeploy Now. Docker. Monorepo with an Express API and a Next.js App deployed with Docker utilizing turbo prune examples/with-docker. Start Building. Monorepo with Changesets. Simple Next.js monorepo preconfigured to publish packages via Changesets examples/with-changesets. Start Building. Non-Monorepo. Example of using Turborepo in a single project without workspaces examples/non-monorepo. Start Building. Prisma. Monorepo with a Next.js App fully configured with Prisma examples/with-prisma. Start Building. Rollup. Monorepo with a single Next.js app sharing a UI library bundled with Rollup examples/with-rollup. Start Building. Tailwind CSS. Monorepo with multiple Next.js apps sharing a UI Library all using Tailwind CSS with a shared config examples/with-tailwind. Start Building. Vite. Monorepo with multiple Vanilla JS apps bundled with Vite, sharing a UI Library examples/with-vite. Start Building. Vue/Nuxt. Monorepo with Vue and Nuxt, sharing a UI Library examples/with-vue-nuxt. Start Building. For even more examples and starters, see the Turborepo examples directory on GitHub (opens in a new tab).",
  //     raw_url:
  //       "/server/pages/repo/docs/getting-started/from-example.html",
  //     excerpt:
  //       "Docker. Monorepo with an Express API and a Next.js App deployed with Docker utilizing turbo <mark>prune</mark> examples/with-docker. Start Building. Monorepo with Changesets. Simple Next.js monorepo preconfigured to publish packages via",
  //     sub_results: [
  //       {
  //         title: "Turborepo Examples",
  //         url: "/_next/static/chunks/server/pages/repo/docs/getting-started/from-example.html",
  //         weighted_locations: [
  //           {
  //             weight: 1,
  //             balanced_score: 1199.6843,
  //             location: 131,
  //           },
  //         ],
  //         locations: [131],
  //         excerpt:
  //           "Docker. Monorepo with an Express API and a Next.js App deployed with Docker utilizing turbo <mark>prune</mark> examples/with-docker. Start Building. Monorepo with Changesets. Simple Next.js monorepo preconfigured to publish packages via",
  //       },
  //     ],
  //   },
  // },
  // {
  //   id: "en_6e68179",
  //   score: 0.15380569,
  //   words: [27],
  //   data: {
  //     url: "/_next/static/chunks/server/pages/repo/docs/reference/command-line-reference.html",
  //     content:
  //       'CLI Reference. You can use turbo --help to get information on how to use these command and get additional information about their usage on these pages: run. prune. gen. login. logout. link. unlink. bin. Option Syntax Options can be passed to turbo in different ways. Options that require a value can be passed with an equals sign: --opt=<value> --opt="<value with a space>" They can also be passed with a space between: --opt value --opt "value with a space" Boolean options can be enabled as follows: # To pass true --opt # To pass false --opt=false Global Arguments The following flags apply to all commands. --color Forces the use of color even when the output stream is not considered to be a TTY terminal. This can be used to enable turbo\'s color output for CI runners such as Github Actions which have support for rendering color in their log output. turbo run build --color Alternatively, you can also enable color using the FORCE_COLOR environment variable (borrowed from the supports-color nodejs package (opens in a new tab)). Note that this may also enable additional colored output from the actual tasks themselves if they use supports-color to determine whether or not to output with colored output. declare -x FORCE_COLOR=1 turbo run build --no-color Suppresses the use of color in the output when running turbo in an interactive / TTY session. turbo run build --no-color Alternatively, you can also suppress color using the FORCE_COLOR environment variable (borrowed from the supports-color nodejs package (opens in a new tab)). declare -x FORCE_COLOR=0 turbo run build --no-update-notifier Disables the update notification. This notification will be automatically disabled when running in CI environments, but can also be disabled manually via this flag. turbo run build --no-update-notifier Alternatively, you can also disable the update notification by using either the TURBO_NO_UPDATE_NOTIFIER environment variable, or the NO_UPDATE_NOTIFIER environment variable (borrowed from the update-notifier nodejs package (opens in a new tab)). declare -x TURBO_NO_UPDATE_NOTIFIER=1 turbo run build',
  //     word_count: 325,
  //     filters: {},
  //     meta: {
  //       title: "CLI Reference",
  //     },
  //     anchors: [
  //       {
  //         element: "a",
  //         id: "option-syntax",
  //         text: "",
  //         location: 36,
  //       },
  //       {
  //         element: "a",
  //         id: "global-arguments",
  //         text: "",
  //         location: 97,
  //       },
  //       {
  //         element: "a",
  //         id: "--color",
  //         text: "",
  //         location: 105,
  //       },
  //       {
  //         element: "a",
  //         id: "--no-color",
  //         text: "",
  //         location: 210,
  //       },
  //       {
  //         element: "a",
  //         id: "--no-update-notifier",
  //         text: "",
  //         location: 260,
  //       },
  //     ],
  //     weighted_locations: [
  //       {
  //         weight: 1,
  //         balanced_score: 1199.6843,
  //         location: 27,
  //       },
  //     ],
  //     locations: [27],
  //     raw_content:
  //       'CLI Reference. You can use turbo --help to get information on how to use these command and get additional information about their usage on these pages: run. prune. gen. login. logout. link. unlink. bin. Option Syntax Options can be passed to turbo in different ways. Options that require a value can be passed with an equals sign: --opt=&lt;value&gt; --opt="&lt;value with a space&gt;" They can also be passed with a space between: --opt value --opt "value with a space" Boolean options can be enabled as follows: # To pass true --opt # To pass false --opt=false Global Arguments The following flags apply to all commands. --color Forces the use of color even when the output stream is not considered to be a TTY terminal. This can be used to enable turbo\'s color output for CI runners such as Github Actions which have support for rendering color in their log output. turbo run build --color Alternatively, you can also enable color using the FORCE_COLOR environment variable (borrowed from the supports-color nodejs package (opens in a new tab)). Note that this may also enable additional colored output from the actual tasks themselves if they use supports-color to determine whether or not to output with colored output. declare -x FORCE_COLOR=1 turbo run build --no-color Suppresses the use of color in the output when running turbo in an interactive / TTY session. turbo run build --no-color Alternatively, you can also suppress color using the FORCE_COLOR environment variable (borrowed from the supports-color nodejs package (opens in a new tab)). declare -x FORCE_COLOR=0 turbo run build --no-update-notifier Disables the update notification. This notification will be automatically disabled when running in CI environments, but can also be disabled manually via this flag. turbo run build --no-update-notifier Alternatively, you can also disable the update notification by using either the TURBO_NO_UPDATE_NOTIFIER environment variable, or the NO_UPDATE_NOTIFIER environment variable (borrowed from the update-notifier nodejs package (opens in a new tab)). declare -x TURBO_NO_UPDATE_NOTIFIER=1 turbo run build',
  //     raw_url:
  //       "/server/pages/repo/docs/reference/command-line-reference.html",
  //     excerpt:
  //       "CLI Reference. You can use turbo --help to get information on how to use these command and get additional information about their usage on these pages: run. <mark>prune.</mark> gen. login.",
  //     sub_results: [
  //       {
  //         title: "CLI Reference",
  //         url: "/_next/static/chunks/server/pages/repo/docs/reference/command-line-reference.html",
  //         weighted_locations: [
  //           {
  //             weight: 1,
  //             balanced_score: 1199.6843,
  //             location: 27,
  //           },
  //         ],
  //         locations: [27],
  //         excerpt:
  //           "CLI Reference. You can use turbo --help to get information on how to use these command and get additional information about their usage on these pages: run. <mark>prune.</mark> gen. login.",
  //       },
  //     ],
  //   },
  // },
  // {
  //   id: "en_d81f562",
  //   score: 0.06978743,
  //   words: [
  //     58, 340, 657, 673, 689, 714, 724, 732, 737, 743, 752,
  //     756, 777, 788, 792, 828, 844, 852, 872, 878, 897,
  //     1476,
  //   ],
  //   data: {
  //     url: "/_next/static/chunks/server/pages/blog/turbo-0-4-0.html",
  //     content:
  //       'Turborepo 0.4.0. NameJared PalmerX@jaredpalmer. I\'m excited to announce the release of Turborepo v0.4.0! 10x faster: turbo has been rewritten from the ground up in Go to make it even more blazing fast. Smarter hashing: Improved hashing algorithm now considers resolved dependencies instead of just the contents of the entire root lockfile. Partial lockfiles / sparse installs: Generate a pruned subset of your root lockfile and monorepo that includes only the necessary packages needed for a given target. Fine-grained scheduling: Improved task orchestration and options via pipeline configuration. Better cache control: You can now specify cache outputs on a per-task basis. Rewritten in Go Although I initially prototyped turbo in TypeScript, it became clear that certain items on the roadmap would require better performance. After around a month or so of work, I\'m excited to finally release Go version of the turbo CLI. Not only does it boot in a milliseconds, but the new Go implementation is somewhere between 10x and 100x faster at hashing than the Node.js implementation. With this new foundation (and some features you\'re about to read about), Turborepo can now scale to intergalactic sized projects while remaining blazing fast all thanks to Go\'s awesome concurrency controls. Better Hashing Not only is hashing faster in v0.4.0, but also a lot smarter. The major change is that turbo no longer includes the hash of the contents of the root lockfile in its hasher (the algorithm responsible for determining if a given task exists in the cache or needs to be executed). Instead, turbo now hashes the set of the resolved versions of a package\'s dependencies and devDependencies based on the root lockfile. The old behavior would explode the cache whenever the root lockfile changed in any way. With this new behavior, changing the lockfile will only bust the cache for those package\'s impacted by the added/changed/removed dependencies. While this sounds complicated, again all it means is that when you install/remove/update dependencies from npm, only those packages that are actually impacted by the changes will need to be rebuilt. Experimental: Pruned Workspaces One of our biggest customer pain points/requests has been improving Docker build times when working with large Yarn Workspaces (or really any workspace implementation). The core issue is that workspaces\' best feature--reducing your monorepo to a single lockfile--is also its worst when it comes to Docker layer caching. To help articulate the problem and how turbo now solves it, let\'s look at an example. Say we have a monorepo with Yarn workspaces that includes a set of packages called frontend, admin, ui, and backend. Let\'s also assume that frontend and admin are Next.js applications that both depend on the same internal React component library package ui. Now let\'s also say that backend contains an Express TypeScript REST API that doesn\'t really share much code with any other part of our monorepo. Here\'s what the Dockerfile for the frontend Next.js app might look like: FROM node:alpine AS base RUN apk update WORKDIR /app # Add lockfile and package.jsons FROM base AS builder COPY *.json yarn.lock ./ COPY packages/ui/*.json ./packages/ui/ COPY packages/frontend/*.json ./packages/frontend/ RUN yarn install # Copy source files COPY packages/ui/ ./packages/ui/ COPY packages/frontend/ ./packages/frontend/ # Build RUN yarn --cwd=packages/ui/ build RUN yarn --cwd=packages/frontend/ build # Start the Frontend Next.js application EXPOSE 3000 RUN [\'yarn\', \'--cwd\', \'packages/frontend\', \'start\'] While this works, there are some things that could be a lot better: You manually COPY in the internal packages and files needed to build the target app and need to remember which need to be built first. You COPY the root yarn.lock lockfile into the correct position very early in the Dockerfile, but this lockfile is the lockfile for the entire monorepo. This last issue is especially painful as your monorepo gets larger and larger because any change to this lockfile triggers a nearly full rebuild regardless of whether or not the app is actually impacted by the new/changed dependencies. ....until now. With the all new turbo prune command, you can now fix this nightmare by deterministically generating a sparse/partial monorepo with a pruned lockfile for a target package--without installing your node_modules. Let\'s look at how to use turbo prune inside of Docker. FROM node:alpine AS base RUN apk update && apk add git ## Globally install `turbo` RUN npm i -g turbo # Prune the workspace for the `frontend` app FROM base as pruner WORKDIR /app COPY . . RUN turbo prune frontend --docker # Add pruned lockfile and package.json\'s of the pruned subworkspace FROM base AS installer WORKDIR /app COPY --from=pruner /app/out/json/ . COPY --from=pruner /app/out/yarn.lock ./yarn.lock # Install only the deps needed to build the target RUN yarn install # Copy source code of pruned subworkspace and build FROM base AS builder WORKDIR /app COPY --from=pruner /app/.git ./.git COPY --from=pruner /app/out/full/ . COPY --from=installer /app/ . RUN turbo run build frontend # Start the app FROM builder as runner EXPOSE 3000 RUN [\'yarn\', \'--cwd\', \'packages/frontend\', \'start\'] So what exactly is the output of the turbo prune? A folder called out with the following inside of it: A folder json with the pruned workspace\'s package.jsons. A folder full with the pruned workspace\'s full source code, but only including the internal packages that are needed to build the target. A new pruned lockfile that only contains the pruned subset of the original root lockfile with the dependencies that are actually used by the packages in the pruned workspace. Thanks to the above, Docker can now be set up to only rebuild each application when there is a real reason to do so. So frontend will only rebuild when its source or dependencies (either internal or from npm) have actually changed. Same same for admin and backend. Changes to ui, either to its source code or dependencies, will trigger rebuilds of both frontend and admin, but not backend. While this example seems trivial, just imagine if each app takes up to 20 minutes to build and deploy. These savings really start to add up quickly, especially on large teams. Pipelines To give you even more control over your Turborepo, we\'ve added pipeline to turbo\'s configuration. This new field in lets you specify how the npm scripts in your monorepo relate to each other as well as some additional per-task options. turbo then uses this information to optimally schedule your tasks in your monorepo, collapsing waterfalls that would otherwise exist. Here\'s how it works: // <root>/package.json { "turbo": { "pipeline": { "build": { // This `^` tells `turbo` that this pipeline target relies on a topological target being completed. // In english, this reads as: "this package\'s `build` command depends on its dependencies\' or // devDependencies\' `build` command being completed" "dependsOn": ["^build"] }, "test": { // `dependsOn` without `^` can be used to express the relationships between tasks at the package level. // In English, this reads as: "this package\'s `test` command depends on its `lint` and `build` command first being completed" "dependsOn": ["lint", "build"] }, "lint": {}, "dev": {} } } } The above config would then be interpreted by turbo to optimally schedule execution. What\'s that actually mean? In the past (like Lerna and Nx), turbo could only run tasks in topological order. With the addition of pipelines, turbo now constructs a topological "action" graph in addition to the actual dependency graph which it uses to determine the order in which tasks should be executed with maximum concurrency. The end result is that you no longer waste idle CPU time waiting around for stuff to finish (i.e. no more waterfalls). Improved Cache Control Thanks to pipeline, we now have a great place to open up turbo\'s cache behavior on a per-task basis. Building on the example from above, you can now set cache output conventions across your entire monorepo like so: // <root>/package.json { "turbo": { "pipeline": { "build": { // Cache anything in dist or .next directories emitted by a `build` command "outputs": ["dist/**", ".next/**", "!.next/cache/**"] "dependsOn": ["^build"] }, "test": { // Cache the test coverage report "outputs": ["coverage/**"], "dependsOn": ["lint", "build"] }, "dev": { // Never cache the `dev` command "cache": false }, "lint": {}, } } } Note: Right now, pipeline exists at the project level, but in later releases these will be overridable on per-package basis. What\'s Next? I know this was a lot, but there\'s even more to come. Here\'s what\'s up next on the Turborepo roadmap. A landing page! Remote caching w/ @turborepo/server (opens in a new tab). Build scans, telemetry, and metrics and dependency and task graph visualization. Desktop Console UI (opens in a new tab). Intelligent watch mode. Official build rules for TypeScript, React, Jest, Node.js, Docker, Kubernetes, and more. Credits Iheanyi Ekechukwu (opens in a new tab) for guiding me through the Go ecosystem. Miguel Oller (opens in a new tab) and the team from Makeswift (opens in a new tab) for iterating on the new prune command.',
  //     word_count: 1478,
  //     filters: {},
  //     meta: {
  //       title: "Turborepo 0.4.0",
  //       image:
  //         "/_next/image?url=%2Fimages%2Fpeople%2Fjaredpalmer.jpeg&amp;w=64&amp;q=75",
  //       image_alt: "Jared Palmer",
  //     },
  //     anchors: [
  //       {
  //         element: "a",
  //         id: "rewritten-in-go",
  //         text: "",
  //         location: 103,
  //       },
  //       {
  //         element: "a",
  //         id: "better-hashing",
  //         text: "",
  //         location: 201,
  //       },
  //       {
  //         element: "a",
  //         id: "experimental-pruned-workspaces",
  //         text: "",
  //         location: 342,
  //       },
  //       {
  //         element: "a",
  //         id: "pipelines",
  //         text: "",
  //         location: 1000,
  //       },
  //       {
  //         element: "a",
  //         id: "improved-cache-control",
  //         text: "",
  //         location: 1254,
  //       },
  //       {
  //         element: "a",
  //         id: "whats-next",
  //         text: "",
  //         location: 1373,
  //       },
  //       {
  //         element: "a",
  //         id: "credits",
  //         text: "",
  //         location: 1440,
  //       },
  //     ],
  //     weighted_locations: [
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 58,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 340,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 657,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 673,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 689,
  //       },
  //       {
  //         weight: 0.5,
  //         balanced_score: 299.92108,
  //         location: 714,
  //       },
  //       {
  //         weight: 0.5,
  //         balanced_score: 288,
  //         location: 724,
  //       },
  //       {
  //         weight: 0.5,
  //         balanced_score: 299.92108,
  //         location: 732,
  //       },
  //       {
  //         weight: 0.5,
  //         balanced_score: 299.92108,
  //         location: 737,
  //       },
  //       {
  //         weight: 0.5,
  //         balanced_score: 299.92108,
  //         location: 743,
  //       },
  //       {
  //         weight: 0.25,
  //         balanced_score: 72,
  //         location: 752,
  //       },
  //       {
  //         weight: 0.25,
  //         balanced_score: 72,
  //         location: 756,
  //       },
  //       {
  //         weight: 0.5,
  //         balanced_score: 299.92108,
  //         location: 777,
  //       },
  //       {
  //         weight: 0.25,
  //         balanced_score: 72,
  //         location: 788,
  //       },
  //       {
  //         weight: 0.25,
  //         balanced_score: 72,
  //         location: 792,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 828,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 844,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 852,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 872,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 878,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 897,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 1476,
  //       },
  //     ],
  //     locations: [
  //       58, 340, 657, 673, 689, 714, 724, 732, 737, 743,
  //       752, 756, 777, 788, 792, 828, 844, 852, 872, 878,
  //       897, 1476,
  //     ],
  //     raw_content:
  //       'Turborepo 0.4.0. NameJared PalmerX@jaredpalmer. I\'m excited to announce the release of Turborepo v0.4.0! 10x faster: turbo has been rewritten from the ground up in Go to make it even more blazing fast. Smarter hashing: Improved hashing algorithm now considers resolved dependencies instead of just the contents of the entire root lockfile. Partial lockfiles / sparse installs: Generate a pruned subset of your root lockfile and monorepo that includes only the necessary packages needed for a given target. Fine-grained scheduling: Improved task orchestration and options via pipeline configuration. Better cache control: You can now specify cache outputs on a per-task basis. Rewritten in Go Although I initially prototyped turbo in TypeScript, it became clear that certain items on the roadmap would require better performance. After around a month or so of work, I\'m excited to finally release Go version of the turbo CLI. Not only does it boot in a milliseconds, but the new Go implementation is somewhere between 10x and 100x faster at hashing than the Node.js implementation. With this new foundation (and some features you\'re about to read about), Turborepo can now scale to intergalactic sized projects while remaining blazing fast all thanks to Go\'s awesome concurrency controls. Better Hashing Not only is hashing faster in v0.4.0, but also a lot smarter. The major change is that turbo no longer includes the hash of the contents of the root lockfile in its hasher (the algorithm responsible for determining if a given task exists in the cache or needs to be executed). Instead, turbo now hashes the set of the resolved versions of a package\'s dependencies and devDependencies based on the root lockfile. The old behavior would explode the cache whenever the root lockfile changed in any way. With this new behavior, changing the lockfile will only bust the cache for those package\'s impacted by the added/changed/removed dependencies. While this sounds complicated, again all it means is that when you install/remove/update dependencies from npm, only those packages that are actually impacted by the changes will need to be rebuilt. Experimental: Pruned Workspaces One of our biggest customer pain points/requests has been improving Docker build times when working with large Yarn Workspaces (or really any workspace implementation). The core issue is that workspaces\' best feature--reducing your monorepo to a single lockfile--is also its worst when it comes to Docker layer caching. To help articulate the problem and how turbo now solves it, let\'s look at an example. Say we have a monorepo with Yarn workspaces that includes a set of packages called frontend, admin, ui, and backend. Let\'s also assume that frontend and admin are Next.js applications that both depend on the same internal React component library package ui. Now let\'s also say that backend contains an Express TypeScript REST API that doesn\'t really share much code with any other part of our monorepo. Here\'s what the Dockerfile for the frontend Next.js app might look like: FROM node:alpine AS base RUN apk update WORKDIR /app # Add lockfile and package.jsons FROM base AS builder COPY *.json yarn.lock ./ COPY packages/ui/*.json ./packages/ui/ COPY packages/frontend/*.json ./packages/frontend/ RUN yarn install # Copy source files COPY packages/ui/ ./packages/ui/ COPY packages/frontend/ ./packages/frontend/ # Build RUN yarn --cwd=packages/ui/ build RUN yarn --cwd=packages/frontend/ build # Start the Frontend Next.js application EXPOSE 3000 RUN [\'yarn\', \'--cwd\', \'packages/frontend\', \'start\'] While this works, there are some things that could be a lot better: You manually COPY in the internal packages and files needed to build the target app and need to remember which need to be built first. You COPY the root yarn.lock lockfile into the correct position very early in the Dockerfile, but this lockfile is the lockfile for the entire monorepo. This last issue is especially painful as your monorepo gets larger and larger because any change to this lockfile triggers a nearly full rebuild regardless of whether or not the app is actually impacted by the new/changed dependencies. ....until now. With the all new turbo prune command, you can now fix this nightmare by deterministically generating a sparse/partial monorepo with a pruned lockfile for a target package--without installing your node_modules. Let\'s look at how to use turbo prune inside of Docker. FROM node:alpine AS base RUN apk update && apk add git ## Globally install `turbo` RUN npm i -g turbo # Prune the workspace for the `frontend` app FROM base as pruner WORKDIR /app COPY . . RUN turbo prune frontend --docker # Add pruned lockfile and package.json\'s of the pruned subworkspace FROM base AS installer WORKDIR /app COPY --from=pruner /app/out/json/ . COPY --from=pruner /app/out/yarn.lock ./yarn.lock # Install only the deps needed to build the target RUN yarn install # Copy source code of pruned subworkspace and build FROM base AS builder WORKDIR /app COPY --from=pruner /app/.git ./.git COPY --from=pruner /app/out/full/ . COPY --from=installer /app/ . RUN turbo run build frontend # Start the app FROM builder as runner EXPOSE 3000 RUN [\'yarn\', \'--cwd\', \'packages/frontend\', \'start\'] So what exactly is the output of the turbo prune? A folder called out with the following inside of it: A folder json with the pruned workspace\'s package.jsons. A folder full with the pruned workspace\'s full source code, but only including the internal packages that are needed to build the target. A new pruned lockfile that only contains the pruned subset of the original root lockfile with the dependencies that are actually used by the packages in the pruned workspace. Thanks to the above, Docker can now be set up to only rebuild each application when there is a real reason to do so. So frontend will only rebuild when its source or dependencies (either internal or from npm) have actually changed. Same same for admin and backend. Changes to ui, either to its source code or dependencies, will trigger rebuilds of both frontend and admin, but not backend. While this example seems trivial, just imagine if each app takes up to 20 minutes to build and deploy. These savings really start to add up quickly, especially on large teams. Pipelines To give you even more control over your Turborepo, we\'ve added pipeline to turbo\'s configuration. This new field in lets you specify how the npm scripts in your monorepo relate to each other as well as some additional per-task options. turbo then uses this information to optimally schedule your tasks in your monorepo, collapsing waterfalls that would otherwise exist. Here\'s how it works: // &lt;root&gt;/package.json { "turbo": { "pipeline": { "build": { // This `^` tells `turbo` that this pipeline target relies on a topological target being completed. // In english, this reads as: "this package\'s `build` command depends on its dependencies\' or // devDependencies\' `build` command being completed" "dependsOn": ["^build"] }, "test": { // `dependsOn` without `^` can be used to express the relationships between tasks at the package level. // In English, this reads as: "this package\'s `test` command depends on its `lint` and `build` command first being completed" "dependsOn": ["lint", "build"] }, "lint": {}, "dev": {} } } } The above config would then be interpreted by turbo to optimally schedule execution. What\'s that actually mean? In the past (like Lerna and Nx), turbo could only run tasks in topological order. With the addition of pipelines, turbo now constructs a topological "action" graph in addition to the actual dependency graph which it uses to determine the order in which tasks should be executed with maximum concurrency. The end result is that you no longer waste idle CPU time waiting around for stuff to finish (i.e. no more waterfalls). Improved Cache Control Thanks to pipeline, we now have a great place to open up turbo\'s cache behavior on a per-task basis. Building on the example from above, you can now set cache output conventions across your entire monorepo like so: // &lt;root&gt;/package.json { "turbo": { "pipeline": { "build": { // Cache anything in dist or .next directories emitted by a `build` command "outputs": ["dist/**", ".next/**", "!.next/cache/**"] "dependsOn": ["^build"] }, "test": { // Cache the test coverage report "outputs": ["coverage/**"], "dependsOn": ["lint", "build"] }, "dev": { // Never cache the `dev` command "cache": false }, "lint": {}, } } } Note: Right now, pipeline exists at the project level, but in later releases these will be overridable on per-package basis. What\'s Next? I know this was a lot, but there\'s even more to come. Here\'s what\'s up next on the Turborepo roadmap. A landing page! Remote caching w/ @turborepo/server (opens in a new tab). Build scans, telemetry, and metrics and dependency and task graph visualization. Desktop Console UI (opens in a new tab). Intelligent watch mode. Official build rules for TypeScript, React, Jest, Node.js, Docker, Kubernetes, and more. Credits Iheanyi Ekechukwu (opens in a new tab) for guiding me through the Go ecosystem. Miguel Oller (opens in a new tab) and the team from Makeswift (opens in a new tab) for iterating on the new prune command.',
  //     raw_url: "/server/pages/blog/turbo-0-4-0.html",
  //     excerpt:
  //       "# <mark>Prune</mark> the workspace for the `frontend` app FROM base as <mark>pruner</mark> WORKDIR /app COPY . . RUN turbo <mark>prune</mark> frontend --docker # Add <mark>pruned</mark> lockfile and package.json's of the",
  //     sub_results: [
  //       {
  //         title: "Turborepo 0.4.0",
  //         url: "/_next/static/chunks/server/pages/blog/turbo-0-4-0.html",
  //         weighted_locations: [
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 58,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 340,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 657,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 673,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 689,
  //           },
  //           {
  //             weight: 0.5,
  //             balanced_score: 299.92108,
  //             location: 714,
  //           },
  //           {
  //             weight: 0.5,
  //             balanced_score: 288,
  //             location: 724,
  //           },
  //           {
  //             weight: 0.5,
  //             balanced_score: 299.92108,
  //             location: 732,
  //           },
  //           {
  //             weight: 0.5,
  //             balanced_score: 299.92108,
  //             location: 737,
  //           },
  //           {
  //             weight: 0.5,
  //             balanced_score: 299.92108,
  //             location: 743,
  //           },
  //           {
  //             weight: 0.25,
  //             balanced_score: 72,
  //             location: 752,
  //           },
  //           {
  //             weight: 0.25,
  //             balanced_score: 72,
  //             location: 756,
  //           },
  //           {
  //             weight: 0.5,
  //             balanced_score: 299.92108,
  //             location: 777,
  //           },
  //           {
  //             weight: 0.25,
  //             balanced_score: 72,
  //             location: 788,
  //           },
  //           {
  //             weight: 0.25,
  //             balanced_score: 72,
  //             location: 792,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 828,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 844,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 852,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 872,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 878,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 897,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 1476,
  //           },
  //         ],
  //         locations: [
  //           58, 340, 657, 673, 689, 714, 724, 732, 737, 743,
  //           752, 756, 777, 788, 792, 828, 844, 852, 872,
  //           878, 897, 1476,
  //         ],
  //         excerpt:
  //           "# <mark>Prune</mark> the workspace for the `frontend` app FROM base as <mark>pruner</mark> WORKDIR /app COPY . . RUN turbo <mark>prune</mark> frontend --docker # Add <mark>pruned</mark> lockfile and package.json's of the",
  //       },
  //     ],
  //   },
  // },
  // {
  //   id: "en_1e1254e",
  //   score: 0.018174391,
  //   words: [42, 81, 88, 305, 330, 344, 357, 361, 408, 419],
  //   data: {
  //     url: "/_next/static/chunks/server/pages/blog/turbo-1-5-0.html",
  //     content:
  //       'Turborepo 1.5. Monday, September 19th, 2022. NameMatt PocockX@mattpocockuk. NameGreg SoltisX@gsoltis. NameNathan HammondX@nathanhammond. NameTom KnickmanX@tknickman. NameAnthony ShewX@anthonysheww. NameJared PalmerX@jaredpalmer. NameMehul KarX@mehulkar. NameChris Olszewski. Turborepo 1.5 is a huge leap forward for our documentation and DX, as well as bringing big improvements to turbo prune: The Monorepo Handbook: We\'ve built the missing manual for your monorepo - a guide on workspaces, code sharing, integrating common tools and much more. Drop the run: turbo run <task> can now be shortened to turbo <task> turbo prune now supports pnpm and yarn 2+: Pruning your monorepo is now supported in pnpm and yarn@berry. Improved environment variables in turbo.json: Environment variables are now first-class citizens in your Turborepo pipeline configuration. Changes to package.json hashing: We\'ve improved how we hash package.json when running tasks. Update today by running npm install turbo@latest. The Monorepo Handbook Setting up a monorepo for the first time often means navigating a lot of new concepts. You\'ll need to understand workspaces, package installation, sharing code and dependency management - and a lot more. This often meant that folks who wanted to set up a monorepo from scratch had to piece information together from different documentation sites. First pnpm, then tsup, then back to changesets, then back to Turborepo for dessert. We want to fill this gap with the Monorepo Handbook. We\'ve built guides on how to integrate all the tools you\'ll need to make ship happen with your monorepo, including guides on: Installing Packages. Linting. Development Tasks. Building Apps. Publishing Packages. Drop the run You can now run tasks with the Turborepo CLI using turbo <task>. - turbo run build + turbo build - turbo run lint build test + turbo lint build test If your task name conflicts with a built-in turbo subcommand, we\'ll run our subcommand instead. That means you shouldn\'t name your tasks things like prune, run, or login - since those are built-in subcommands. turbo run <task> will continue to work, and there are no plans to deprecate it. Prune now supported on pnpm and yarn 2+ We\'re delighted to announce that turbo prune now supports in pnpm, yarn, and yarn 2+. You can use turbo prune to create a pruned subset of your monorepo with a dedicated lockfile--with the correct dependencies needed for a given target application and its dependencies. This is especially useful for using efficiently Turborepo within a Docker image. As part of the new handbook, we\'ve also added a section on using turbo prune to build docker images. Check out our previous blog on prune to learn more. Environment variables in turbo.json We\'ve introduced two new keys to turbo.json - env and globalEnv. These allow environment variables to be configured separately from tasks: { "globalDependencies": [ - "$DATABASE_URL" ], + "globalEnv": [ + "DATABASE_URL" + ], "pipeline": { "build": { "dependsOn": [ - "$BUILD_ENV" ], + "env": [ + "BUILD_ENV" + ] } } } globalEnv and env allow you to specify a list of environment variables without $ prefixes. This makes the configuration file significantly easier to read. Read more in our updated docs. To help migrate from the previous syntax, we\'ve prepared a codemod. You can run npx @turbo/codemod migrate-env-var-dependencies. This work builds on the automatic env variable detection we added in 1.4.0. Changes to package.json hashing The package.json file in each workspace is now always considered an input for tasks in that workspace. This means that if you change the definition for a task in package.json, we want to invalidate any caches from the previous definition. This also counts for the package.json in the root. Changes to the root package.json will invalidate tasks in the root workspace. This helps make Turborepo\'s cache a bit smarter, and less likely to trip up when task definitions change. Community Since releasing Turborepo v1.4 in August, we\'ve seen incredible adoption and community growth: 9.5k+ GitHub Stars (opens in a new tab). 440k weekly NPM downloads (opens in a new tab). 15 years of compute time saved through Remote Caching on Vercel (opens in a new tab), saving over a 1 year per week, up 2x since July. Turborepo is the result of the combined work of all of our contributors including our core team. This release was brought to you by the contributions of: @7flash, @afady, @alexander-young, @atilafassina, @bguedes-moz, @bobaaaaa, @brunojppb, @chris-olszewski, @DoctorJohn, @erj826, @futantan, @gsoltis, @HosseinAgha, @ivov, @jaredpalmer, @joelhooks, @knownasnaffy, @laurentlucian, @leerob, @MarceloAlves, @mattpocock, @mauricekleine, @mehulkar, @Misikir, @nareshbhatia, @nathanhammond, @pakaponk, @PhentomPT, @renovate, @ruisaraiva19, @samuelhorn, @shemayas, @shuding, @t-i-0414, @theurgi, @tknickman, @yanmao-cc, and more! Thank you for your continued support, feedback, and collaboration to make Turborepo your build tool of choice.',
  //     word_count: 764,
  //     filters: {},
  //     meta: {
  //       title: "Turborepo 1.5",
  //       image:
  //         "/_next/image?url=%2Fimages%2Fpeople%2Fmattpocock.jpeg&amp;w=64&amp;q=75",
  //       image_alt: "Matt Pocock",
  //     },
  //     anchors: [
  //       {
  //         element: "a",
  //         id: "the-monorepo-handbook",
  //         text: "",
  //         location: 137,
  //       },
  //       {
  //         element: "a",
  //         id: "drop-the-run",
  //         text: "",
  //         location: 251,
  //       },
  //       {
  //         element: "a",
  //         id: "prune-now-supported-on-pnpm-and-yarn-2",
  //         text: "",
  //         location: 338,
  //       },
  //       {
  //         element: "a",
  //         id: "environment-variables-in-turbojson",
  //         text: "",
  //         location: 427,
  //       },
  //       {
  //         element: "a",
  //         id: "changes-to-packagejson-hashing",
  //         text: "",
  //         location: 544,
  //       },
  //       {
  //         element: "a",
  //         id: "community",
  //         text: "",
  //         location: 624,
  //       },
  //     ],
  //     weighted_locations: [
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 42,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 81,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 88,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 305,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 330,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 344,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 357,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 361,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 408,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 419,
  //       },
  //     ],
  //     locations: [
  //       42, 81, 88, 305, 330, 344, 357, 361, 408, 419,
  //     ],
  //     raw_content:
  //       'Turborepo 1.5. Monday, September 19th, 2022. NameMatt PocockX@mattpocockuk. NameGreg SoltisX@gsoltis. NameNathan HammondX@nathanhammond. NameTom KnickmanX@tknickman. NameAnthony ShewX@anthonysheww. NameJared PalmerX@jaredpalmer. NameMehul KarX@mehulkar. NameChris Olszewski. Turborepo 1.5 is a huge leap forward for our documentation and DX, as well as bringing big improvements to turbo prune: The Monorepo Handbook: We\'ve built the missing manual for your monorepo - a guide on workspaces, code sharing, integrating common tools and much more. Drop the run: turbo run &lt;task&gt; can now be shortened to turbo &lt;task&gt; turbo prune now supports pnpm and yarn 2+: Pruning your monorepo is now supported in pnpm and yarn@berry. Improved environment variables in turbo.json: Environment variables are now first-class citizens in your Turborepo pipeline configuration. Changes to package.json hashing: We\'ve improved how we hash package.json when running tasks. Update today by running npm install turbo@latest. The Monorepo Handbook Setting up a monorepo for the first time often means navigating a lot of new concepts. You\'ll need to understand workspaces, package installation, sharing code and dependency management - and a lot more. This often meant that folks who wanted to set up a monorepo from scratch had to piece information together from different documentation sites. First pnpm, then tsup, then back to changesets, then back to Turborepo for dessert. We want to fill this gap with the Monorepo Handbook. We\'ve built guides on how to integrate all the tools you\'ll need to make ship happen with your monorepo, including guides on: Installing Packages. Linting. Development Tasks. Building Apps. Publishing Packages. Drop the run You can now run tasks with the Turborepo CLI using turbo &lt;task&gt;. - turbo run build + turbo build - turbo run lint build test + turbo lint build test If your task name conflicts with a built-in turbo subcommand, we\'ll run our subcommand instead. That means you shouldn\'t name your tasks things like prune, run, or login - since those are built-in subcommands. turbo run &lt;task&gt; will continue to work, and there are no plans to deprecate it. Prune now supported on pnpm and yarn 2+ We\'re delighted to announce that turbo prune now supports in pnpm, yarn, and yarn 2+. You can use turbo prune to create a pruned subset of your monorepo with a dedicated lockfile--with the correct dependencies needed for a given target application and its dependencies. This is especially useful for using efficiently Turborepo within a Docker image. As part of the new handbook, we\'ve also added a section on using turbo prune to build docker images. Check out our previous blog on prune to learn more. Environment variables in turbo.json We\'ve introduced two new keys to turbo.json - env and globalEnv. These allow environment variables to be configured separately from tasks: { "globalDependencies": [ - "$DATABASE_URL" ], + "globalEnv": [ + "DATABASE_URL" + ], "pipeline": { "build": { "dependsOn": [ - "$BUILD_ENV" ], + "env": [ + "BUILD_ENV" + ] } } } globalEnv and env allow you to specify a list of environment variables without $ prefixes. This makes the configuration file significantly easier to read. Read more in our updated docs. To help migrate from the previous syntax, we\'ve prepared a codemod. You can run npx @turbo/codemod migrate-env-var-dependencies. This work builds on the automatic env variable detection we added in 1.4.0. Changes to package.json hashing The package.json file in each workspace is now always considered an input for tasks in that workspace. This means that if you change the definition for a task in package.json, we want to invalidate any caches from the previous definition. This also counts for the package.json in the root. Changes to the root package.json will invalidate tasks in the root workspace. This helps make Turborepo\'s cache a bit smarter, and less likely to trip up when task definitions change. Community Since releasing Turborepo v1.4 in August, we\'ve seen incredible adoption and community growth: 9.5k+ GitHub Stars (opens in a new tab). 440k weekly NPM downloads (opens in a new tab). 15 years of compute time saved through Remote Caching on Vercel (opens in a new tab), saving over a 1 year per week, up 2x since July. Turborepo is the result of the combined work of all of our contributors including our core team. This release was brought to you by the contributions of: @7flash, @afady, @alexander-young, @atilafassina, @bguedes-moz, @bobaaaaa, @brunojppb, @chris-olszewski, @DoctorJohn, @erj826, @futantan, @gsoltis, @HosseinAgha, @ivov, @jaredpalmer, @joelhooks, @knownasnaffy, @laurentlucian, @leerob, @MarceloAlves, @mattpocock, @mauricekleine, @mehulkar, @Misikir, @nareshbhatia, @nathanhammond, @pakaponk, @PhentomPT, @renovate, @ruisaraiva19, @samuelhorn, @shemayas, @shuding, @t-i-0414, @theurgi, @tknickman, @yanmao-cc, and more! Thank you for your continued support, feedback, and collaboration to make Turborepo your build tool of choice.',
  //     raw_url: "/server/pages/blog/turbo-1-5-0.html",
  //     excerpt:
  //       "deprecate it. <mark>Prune</mark> now supported on pnpm and yarn 2+ We're delighted to announce that turbo <mark>prune</mark> now supports in pnpm, yarn, and yarn 2+. You can use turbo <mark>prune</mark>",
  //     sub_results: [
  //       {
  //         title: "Turborepo 1.5",
  //         url: "/_next/static/chunks/server/pages/blog/turbo-1-5-0.html",
  //         weighted_locations: [
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 42,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 81,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 88,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 305,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 330,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 344,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 357,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 361,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 408,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 419,
  //           },
  //         ],
  //         locations: [
  //           42, 81, 88, 305, 330, 344, 357, 361, 408, 419,
  //         ],
  //         excerpt:
  //           "deprecate it. <mark>Prune</mark> now supported on pnpm and yarn 2+ We're delighted to announce that turbo <mark>prune</mark> now supports in pnpm, yarn, and yarn 2+. You can use turbo <mark>prune</mark>",
  //       },
  //     ],
  //   },
  // },
  // {
  //   id: "en_aa1aa3f",
  //   score: 0.012087257,
  //   words: [63, 67, 589, 605, 617, 625, 649, 690],
  //   data: {
  //     url: "/_next/static/chunks/server/pages/blog/turbo-1-6-0.html",
  //     content:
  //       'Turborepo 1.6. Friday, October 21st, 2022. NameMatt PocockX@mattpocockuk. NameGreg SoltisX@gsoltis. NameNathan HammondX@nathanhammond. NameTom KnickmanX@tknickman. NameAnthony ShewX@anthonysheww. NameJared PalmerX@jaredpalmer. NameMehul KarX@mehulkar. NameChris Olszewski. Turborepo 1.6 changes the game for Turborepo - you can now use it in any project. Turborepo in non-monorepos: Seeing slow builds on your project? You can now use Turborepo to speed up builds in any codebase with a package.json. turbo prune now supports npm: Pruning your monorepo is now supported in monorepos using npm, completing support for all major workspace managers. Faster caching: We\'ve improved the way we handle local file writes, meaning a big speed-up of Turborepo\'s cache. Update today by running npm install turbo@latest. Any codebase can use Turborepo Turborepo helps speed up tasks in your codebase. Until now, we\'d built Turborepo specifically for monorepos - codebases which contain multiple applications and packages. Turborepo is fantastic in monorepos because they have so many tasks to handle. Each package and app needs to be built, linted, and tested. But we got to thinking: lots of codebases that aren\'t monorepos run plenty of tasks. Most CI/CD processes do a lot of duplicated work that would benefit from a cache. So we\'re excited to announce that any codebase can now use Turborepo. Try it out now by starting from the example (opens in a new tab), or by adding Turborepo to an existing project: Add Turborepo to your project Install turbo: npm yarn pnpm. npm install turbo --save-dev Add a turbo.json file at the base of your new repository: Next.js Vite. { "pipeline": { "build": { "outputs": [".next/**", "!.next/cache/**"] }, "lint": { "outputs": [] } } } Try running build and lint with turbo: turbo build lint Congratulations - you just ran your first build with turbo. You can try: Running through the full Quickstart. Check out our updated Core Concepts docs to understand what makes Turborepo special. When should I use Turborepo? Turborepo being available for non-monorepos opens up a lot of new use cases. But when is it at its best? When scripts depend on each other You should use turbo to run your package.json scripts. If you\'ve got multiple scripts which all rely on each other, you can express them as Turborepo tasks: { "pipeline": { "build": { "outputs": ["dist/**"] }, "lint": { // \'build\' should be run before \'lint\' "dependsOn": ["build"] }, "test": { // \'build\' should be run before \'test\' "dependsOn": ["build"] } } } Then, you can run: turbo run lint test Because you\'ve said that build should be run before lint and test, it\'ll automatically run build for you when you run lint or test. Not only that, but it\'ll figure out the optimal schedule for you. Head to our core concepts doc on optimizing for speed. When you want to run tasks in parallel Imagine you\'re running a Next.js (opens in a new tab) app, and also running the Tailwind CLI (opens in a new tab). You might have two scripts - dev and dev:css: { "scripts": { "dev": "next", "dev:css": "tailwindcss -i ./src/input.css -o ./dist/output.css --watch" } } Without anything being added to your turbo.json, you can run: turbo run dev dev:css Just like tools like concurrently (opens in a new tab), Turborepo will automatically run the two scripts in parallel. This is extremely useful for dev mode, but can also be used to speed up tasks on CI - imagine you have multiple scripts to run: turbo run lint unit:test e2e:test integration:test Turborepo will figure out the fastest possible way to run all your tasks in parallel. Prune now supported on npm Over the last several releases, we\'ve been adding support for turbo prune on different workspace managers. This has been a challenge - turbo prune creates a subset of your monorepo, including pruning the dependencies in your lockfile. This means we\'ve had to implement logic for each workspace manager separately. We\'re delighted to announce that turbo prune now works for npm, completing support for all major package managers. This means that if your monorepo uses npm, yarn, yarn 2+ or pnpm, you\'ll be able to deploy to Docker with ease. Check out our previous blog on turbo prune to learn more. Performance improvements in the cache Before 1.6, Turborepo\'s local cache was a recursive copy of files on the system to another place on disk. This was slow. It meant that for every file that we needed to cache, we\'d need to perform six system calls: open, read, and close on the source file; open, write, and close on the destination file. In 1.6, we\'ve cut that nearly in half. Now, when creating a cache, we create a single .tar file (one open), we write to it in 1mb chunks (batched writes), and then close it (one close). The halving of system calls also happens on the way back out of cache. And we didn\'t stop there. Over the past month we\'ve invested significantly in our build toolchain to enable CGO which unlocks usage of best-in-class libraries written in C. This enabled us to adopt Zstandard (opens in a new tab)\'s libzstd for compression which gets us an algorithmic 3x performance improvement for compression. After all of these changes we\'re regularly seeing performance improvements of more than 2x on local cache creation and more than 3x on remote cache creation. This gets even better the bigger your repository is, or the slower your device is (looking at you, CI). This means we\'ve been able to deliver performance wins precisely to those who needed it the most.',
  //     word_count: 919,
  //     filters: {},
  //     meta: {
  //       title: "Turborepo 1.6",
  //       image:
  //         "/_next/image?url=%2Fimages%2Fpeople%2Fmattpocock.jpeg&amp;w=64&amp;q=75",
  //       image_alt: "Matt Pocock",
  //     },
  //     anchors: [
  //       {
  //         element: "a",
  //         id: "any-codebase-can-use-turborepo",
  //         text: "",
  //         location: 114,
  //       },
  //       {
  //         element: "a",
  //         id: "add-turborepo-to-your-project",
  //         text: "",
  //         location: 231,
  //       },
  //       {
  //         element: "button",
  //         id: "headlessui-tabs-tab-:Rm7j9d6:",
  //         text: "npm",
  //         location: 233,
  //       },
  //       {
  //         element: "button",
  //         id: "headlessui-tabs-tab-:R167j9d6:",
  //         text: "yarn",
  //         location: 234,
  //       },
  //       {
  //         element: "button",
  //         id: "headlessui-tabs-tab-:R1m7j9d6:",
  //         text: "pnpm",
  //         location: 235,
  //       },
  //       {
  //         element: "div",
  //         id: "headlessui-tabs-panel-:Rq7j9d6:",
  //         text: "",
  //         location: 236,
  //       },
  //       {
  //         element: "span",
  //         id: "headlessui-tabs-panel-:R1a7j9d6:",
  //         text: "",
  //         location: 240,
  //       },
  //       {
  //         element: "span",
  //         id: "headlessui-tabs-panel-:R1q7j9d6:",
  //         text: "",
  //         location: 240,
  //       },
  //       {
  //         element: "button",
  //         id: "headlessui-tabs-tab-:Rm8j9d6:",
  //         text: "Next.js",
  //         location: 251,
  //       },
  //       {
  //         element: "button",
  //         id: "headlessui-tabs-tab-:R168j9d6:",
  //         text: "Vite",
  //         location: 252,
  //       },
  //       {
  //         element: "div",
  //         id: "headlessui-tabs-panel-:Rq8j9d6:",
  //         text: "",
  //         location: 253,
  //       },
  //       {
  //         element: "span",
  //         id: "headlessui-tabs-panel-:R1a8j9d6:",
  //         text: "",
  //         location: 269,
  //       },
  //       {
  //         element: "a",
  //         id: "when-should-i-use-turborepo",
  //         text: "",
  //         location: 315,
  //       },
  //       {
  //         element: "a",
  //         id: "when-scripts-depend-on-each-other",
  //         text: "",
  //         location: 341,
  //       },
  //       {
  //         element: "a",
  //         id: "when-you-want-to-run-tasks-in-parallel",
  //         text: "",
  //         location: 464,
  //       },
  //       {
  //         element: "a",
  //         id: "prune-now-supported-on-npm",
  //         text: "",
  //         location: 594,
  //       },
  //       {
  //         element: "a",
  //         id: "performance-improvements-in-the-cache",
  //         text: "",
  //         location: 699,
  //       },
  //     ],
  //     weighted_locations: [
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 63,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 67,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 589,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 605,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 617,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 625,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 649,
  //       },
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 690,
  //       },
  //     ],
  //     locations: [63, 67, 589, 605, 617, 625, 649, 690],
  //     raw_content:
  //       'Turborepo 1.6. Friday, October 21st, 2022. NameMatt PocockX@mattpocockuk. NameGreg SoltisX@gsoltis. NameNathan HammondX@nathanhammond. NameTom KnickmanX@tknickman. NameAnthony ShewX@anthonysheww. NameJared PalmerX@jaredpalmer. NameMehul KarX@mehulkar. NameChris Olszewski. Turborepo 1.6 changes the game for Turborepo - you can now use it in any project. Turborepo in non-monorepos: Seeing slow builds on your project? You can now use Turborepo to speed up builds in any codebase with a package.json. turbo prune now supports npm: Pruning your monorepo is now supported in monorepos using npm, completing support for all major workspace managers. Faster caching: We\'ve improved the way we handle local file writes, meaning a big speed-up of Turborepo\'s cache. Update today by running npm install turbo@latest. Any codebase can use Turborepo Turborepo helps speed up tasks in your codebase. Until now, we\'d built Turborepo specifically for monorepos - codebases which contain multiple applications and packages. Turborepo is fantastic in monorepos because they have so many tasks to handle. Each package and app needs to be built, linted, and tested. But we got to thinking: lots of codebases that aren\'t monorepos run plenty of tasks. Most CI/CD processes do a lot of duplicated work that would benefit from a cache. So we\'re excited to announce that any codebase can now use Turborepo. Try it out now by starting from the example (opens in a new tab), or by adding Turborepo to an existing project: Add Turborepo to your project Install turbo: npm yarn pnpm. npm install turbo --save-dev Add a turbo.json file at the base of your new repository: Next.js Vite. { "pipeline": { "build": { "outputs": [".next/**", "!.next/cache/**"] }, "lint": { "outputs": [] } } } Try running build and lint with turbo: turbo build lint Congratulations - you just ran your first build with turbo. You can try: Running through the full Quickstart. Check out our updated Core Concepts docs to understand what makes Turborepo special. When should I use Turborepo? Turborepo being available for non-monorepos opens up a lot of new use cases. But when is it at its best? When scripts depend on each other You should use turbo to run your package.json scripts. If you\'ve got multiple scripts which all rely on each other, you can express them as Turborepo tasks: { "pipeline": { "build": { "outputs": ["dist/**"] }, "lint": { // \'build\' should be run before \'lint\' "dependsOn": ["build"] }, "test": { // \'build\' should be run before \'test\' "dependsOn": ["build"] } } } Then, you can run: turbo run lint test Because you\'ve said that build should be run before lint and test, it\'ll automatically run build for you when you run lint or test. Not only that, but it\'ll figure out the optimal schedule for you. Head to our core concepts doc on optimizing for speed. When you want to run tasks in parallel Imagine you\'re running a Next.js (opens in a new tab) app, and also running the Tailwind CLI (opens in a new tab). You might have two scripts - dev and dev:css: { "scripts": { "dev": "next", "dev:css": "tailwindcss -i ./src/input.css -o ./dist/output.css --watch" } } Without anything being added to your turbo.json, you can run: turbo run dev dev:css Just like tools like concurrently (opens in a new tab), Turborepo will automatically run the two scripts in parallel. This is extremely useful for dev mode, but can also be used to speed up tasks on CI - imagine you have multiple scripts to run: turbo run lint unit:test e2e:test integration:test Turborepo will figure out the fastest possible way to run all your tasks in parallel. Prune now supported on npm Over the last several releases, we\'ve been adding support for turbo prune on different workspace managers. This has been a challenge - turbo prune creates a subset of your monorepo, including pruning the dependencies in your lockfile. This means we\'ve had to implement logic for each workspace manager separately. We\'re delighted to announce that turbo prune now works for npm, completing support for all major package managers. This means that if your monorepo uses npm, yarn, yarn 2+ or pnpm, you\'ll be able to deploy to Docker with ease. Check out our previous blog on turbo prune to learn more. Performance improvements in the cache Before 1.6, Turborepo\'s local cache was a recursive copy of files on the system to another place on disk. This was slow. It meant that for every file that we needed to cache, we\'d need to perform six system calls: open, read, and close on the source file; open, write, and close on the destination file. In 1.6, we\'ve cut that nearly in half. Now, when creating a cache, we create a single .tar file (one open), we write to it in 1mb chunks (batched writes), and then close it (one close). The halving of system calls also happens on the way back out of cache. And we didn\'t stop there. Over the past month we\'ve invested significantly in our build toolchain to enable CGO which unlocks usage of best-in-class libraries written in C. This enabled us to adopt Zstandard (opens in a new tab)\'s libzstd for compression which gets us an algorithmic 3x performance improvement for compression. After all of these changes we\'re regularly seeing performance improvements of more than 2x on local cache creation and more than 3x on remote cache creation. This gets even better the bigger your repository is, or the slower your device is (looking at you, CI). This means we\'ve been able to deliver performance wins precisely to those who needed it the most.',
  //     raw_url: "/server/pages/blog/turbo-1-6-0.html",
  //     excerpt:
  //       "parallel. <mark>Prune</mark> now supported on npm Over the last several releases, we've been adding support for turbo <mark>prune</mark> on different workspace managers. This has been a challenge - turbo <mark>prune</mark>",
  //     sub_results: [
  //       {
  //         title: "Turborepo 1.6",
  //         url: "/_next/static/chunks/server/pages/blog/turbo-1-6-0.html",
  //         weighted_locations: [
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 63,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 67,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 589,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 605,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 617,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 625,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 649,
  //           },
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 690,
  //           },
  //         ],
  //         locations: [63, 67, 589, 605, 617, 625, 649, 690],
  //         excerpt:
  //           "parallel. <mark>Prune</mark> now supported on npm Over the last several releases, we've been adding support for turbo <mark>prune</mark> on different workspace managers. This has been a challenge - turbo <mark>prune</mark>",
  //       },
  //     ],
  //   },
  // },
  // {
  //   id: "en_a17f30a",
  //   score: 0.0013111648,
  //   words: [890],
  //   data: {
  //     url: "/_next/static/chunks/server/pages/blog/turbo-1-2-0.html",
  //     content:
  //       "Turborepo 1.2. Friday, April 8th, 2022. NameJared PalmerX@jaredpalmer. NameBecca Z.X@becca__z. NameGaspar GarciaX@gaspargarcia_. NameGreg SoltisX@gsoltis. Since releasing Turborepo v1.1 in late January, we've seen incredible adoption and community growth: 6.5k+ GitHub Stars (opens in a new tab). 140k+ weekly npm downloads (doubling since our last blog post for v1.1). 95+ OSS contributors. 900+ members of the Turborepo Community Discord (opens in a new tab). 1.6 years of Time Saved through Remote Caching on Vercel, saving more than 2.5 months every week. We've further improved ergonomics, observability, and security with Turborepo v1.2 featuring: New Task Filtering API: --filter adds more powerful task filtering capabilities to turbo run. Human-readable and JSON dry runs: --dry-run flag can print out information about a turbo run without executing any tasks, in both human and JSON-parse friendly formats. Improved Internal Scheduler and Graph: We refactored turbo 's internal scheduler and graph to be more ergonomic and predictable. Enhanced Remote Cache Security: Cryptographically sign remote cache artifacts with your own secret key. Update today by running npm install turbo@latest. After running turbo run for the first time, you'll see instructions about how to use @turbo/codemod to run automatic migrations for v1.2. New Task Filtering API We are excited to release one of our most requested features: the ability to expressively filter tasks through a --filter flag. The --filter flag is the much more powerful successor to the current combination of --scope, --include-dependencies, --since, and --no-deps flags. With --filter you can tell turbo to restrict executing commands to a subset of matched packages in your monorepo based on name, folder, or even if it has changed since a git commit ref. Take a look at some examples of what you can accomplish with the new --filter command: --filter=<package_name> - match by exact package name or glob pattern. --filter=...<package_name>- match by package name/glob and include all dependent packages of matches. --filter=...^<package_name>- match by package name/glob and include all dependent packages of matches, but exclude the matches themselves. --filter=<package_name>... - match by package name/glob and include all the matched packages' dependencies. --filter=<package_name>^... - match by package name/glob and include all matched package dependencies, but exclude the matches themselves. --filter={./path/to/package} - match by path or filesystem glob pattern. --filter=[origin/main] - match by changed packages since a git commit ref. You can use multiple filters together to get even more granular filtering as well as combine each part of the above patterns {}, [] , ^ , and ... to express more complex behavior. For example, if you had an app located in ./apps/web directory with local packages used as dependencies, and a Turborepo pipeline where test depends on ^build topologically, running: turbo run test --filter={./apps/web}[HEAD^1]^... would tell turbo to ensure dependencies are built and to run the test script in all of the local dependencies of the app located in ./apps/web, not including that app itself, if the app has changed since HEAD^1. For more details and examples, refer to the new filtering documentation. Debug and Automate with --dry-run You can now see the impact of turbo run without actually executing any commands by appending either --dry-run or --dry-run=json to any turbo run command. This will result in either human or JSON output. Dry runs are incredibly useful for two situations: Debugging and testing run options. Using turbo filtering and task graphs for building automations. We hope that this will improve visibility into what turbo is doing, speeding up debugging, and make it easier to leverage turbo in dynamic CI/CD systems. Improved Internal Scheduler and Graph When using turbo run, every package.json task is added to an internal graph to map dependencies based on the inferred relationships defined in your Turborepo pipeline. This task graph allows Turborepo to efficiently schedule incremental concurrent task running and cache task outputs for later use. We have made major improvements to the internal task scheduler and resulting graph structure, resulting in better performance and a better developer experience. For example, in many cases, you will no longer need to use --include-dependencies. Instead, after specifying your task entry points, the new and improved graph will automatically handle this graph resolution on your behalf. Cache Outputs Integrity and Signature Verification You can now configure Turborepo to sign remote cache outputs using HMAC-SHA256 with a secret key before uploading them to the Remote Cache. When Turborepo downloads signed cache artifacts, it will now verify the artifact's integrity and authenticity. Any artifact that fails to verify will be ignored, discarded, and treated as a cache miss by Turborepo. To enable this feature, set the remoteCache options in your turbo.json config file to include signature: true. Then specify your secret key by declaring the TURBO_REMOTE_CACHE_SIGNATURE_KEY environment variable. { \"$schema\": \"[https://turbo.build/schema.json](https://turbo.build/schema.json)\", \"remoteCache\": { // Indicates if signature verification is enabled. \"signature\": true } } Other bug fixes and improvements --sso-team flag now enables teams with SAML tokens to log in through turbo login with correct team permissions. --log-output flag allows you to control what logs are printed to the terminal, and when, allowing you to focus only on what's new. FORCE_COLOR environment variable is now supported. TURBO_FORCE=true environment variable will now force execution. --remote-only and TURBO_REMOTE_ONLY=true will tell turbo to only use Remote Caching. We now show >>> FULL TURBO when there's at least one task attempted. Yarn v2+ with Plug'n'Play (PnP linker) is supported for the turbo run command, but turbo prune is still not fully supported. Fixed regression with chrome tracing if --profile is specified. You can now set concurrency by percentage of CPUs with --concurrency=50% We're hiring! The Turborepo team at Vercel (opens in a new tab) is hiring! We're up to five core team members already this year and are looking to hire even more. We're specifically looking for full-time Senior Build Systems Engineers (opens in a new tab). What's next? Along with seamless incremental adoption/migration and speeding up CI/CD, we've been focusing on improving Turborepo's day-to-day ergonomics, security, and observability. The new --filter flag, signed artifacts, and dry runs are important steps toward those goals. Next up, we'll be focusing an enhanced local development experience, codebase automations, and overall CLI performance. Thank you, contributors Turborepo is the result of the combined work of over 95 individual developers and our core team. This release was brought to you by the contributions of: @gsoltis09, @jaredpalmer, @gaspar09, @shuding, @rajatkulkarni95, @VanTanev, @Kikobeats, @tknickman, @thebanjomatic, @chelkyl, @elado, @finn-orsini, @becca, @weyert, @ekosz.",
  //     word_count: 1059,
  //     filters: {},
  //     meta: {
  //       title: "Turborepo 1.2",
  //       image:
  //         "/_next/image?url=%2Fimages%2Fpeople%2Fjaredpalmer.jpeg&amp;w=64&amp;q=75",
  //       image_alt: "Jared Palmer",
  //     },
  //     anchors: [
  //       {
  //         element: "a",
  //         id: "new-task-filtering-api",
  //         text: "",
  //         location: 197,
  //       },
  //       {
  //         element: "a",
  //         id: "debug-and-automate-with---dry-run",
  //         text: "",
  //         location: 497,
  //       },
  //       {
  //         element: "a",
  //         id: "improved-internal-scheduler-and-graph",
  //         text: "",
  //         location: 584,
  //       },
  //       {
  //         element: "a",
  //         id: "cache-outputs-integrity-and-signature-verification",
  //         text: "",
  //         location: 692,
  //       },
  //       {
  //         element: "a",
  //         id: "other-bug-fixes-and-improvements",
  //         text: "",
  //         location: 797,
  //       },
  //       {
  //         element: "a",
  //         id: "were-hiring",
  //         text: "",
  //         location: 918,
  //       },
  //       {
  //         element: "a",
  //         id: "whats-next",
  //         text: "",
  //         location: 963,
  //       },
  //       {
  //         element: "a",
  //         id: "thank-you-contributors",
  //         text: "",
  //         location: 1017,
  //       },
  //     ],
  //     weighted_locations: [
  //       {
  //         weight: 0.16666666666666666,
  //         balanced_score: 33.324566,
  //         location: 890,
  //       },
  //     ],
  //     locations: [890],
  //     raw_content:
  //       "Turborepo 1.2. Friday, April 8th, 2022. NameJared PalmerX@jaredpalmer. NameBecca Z.X@becca__z. NameGaspar GarciaX@gaspargarcia_. NameGreg SoltisX@gsoltis. Since releasing Turborepo v1.1 in late January, we've seen incredible adoption and community growth: 6.5k+ GitHub Stars (opens in a new tab). 140k+ weekly npm downloads (doubling since our last blog post for v1.1). 95+ OSS contributors. 900+ members of the Turborepo Community Discord (opens in a new tab). 1.6 years of Time Saved through Remote Caching on Vercel, saving more than 2.5 months every week. We've further improved ergonomics, observability, and security with Turborepo v1.2 featuring: New Task Filtering API: --filter adds more powerful task filtering capabilities to turbo run. Human-readable and JSON dry runs: --dry-run flag can print out information about a turbo run without executing any tasks, in both human and JSON-parse friendly formats. Improved Internal Scheduler and Graph: We refactored turbo 's internal scheduler and graph to be more ergonomic and predictable. Enhanced Remote Cache Security: Cryptographically sign remote cache artifacts with your own secret key. Update today by running npm install turbo@latest. After running turbo run for the first time, you'll see instructions about how to use @turbo/codemod to run automatic migrations for v1.2. New Task Filtering API We are excited to release one of our most requested features: the ability to expressively filter tasks through a --filter flag. The --filter flag is the much more powerful successor to the current combination of --scope, --include-dependencies, --since, and --no-deps flags. With --filter you can tell turbo to restrict executing commands to a subset of matched packages in your monorepo based on name, folder, or even if it has changed since a git commit ref. Take a look at some examples of what you can accomplish with the new --filter command: --filter=&lt;package_name&gt; - match by exact package name or glob pattern. --filter=...&lt;package_name&gt;- match by package name/glob and include all dependent packages of matches. --filter=...^&lt;package_name&gt;- match by package name/glob and include all dependent packages of matches, but exclude the matches themselves. --filter=&lt;package_name&gt;... - match by package name/glob and include all the matched packages' dependencies. --filter=&lt;package_name&gt;^... - match by package name/glob and include all matched package dependencies, but exclude the matches themselves. --filter={./path/to/package} - match by path or filesystem glob pattern. --filter=[origin/main] - match by changed packages since a git commit ref. You can use multiple filters together to get even more granular filtering as well as combine each part of the above patterns {}, [] , ^ , and ... to express more complex behavior. For example, if you had an app located in ./apps/web directory with local packages used as dependencies, and a Turborepo pipeline where test depends on ^build topologically, running: turbo run test --filter={./apps/web}[HEAD^1]^... would tell turbo to ensure dependencies are built and to run the test script in all of the local dependencies of the app located in ./apps/web, not including that app itself, if the app has changed since HEAD^1. For more details and examples, refer to the new filtering documentation. Debug and Automate with --dry-run You can now see the impact of turbo run without actually executing any commands by appending either --dry-run or --dry-run=json to any turbo run command. This will result in either human or JSON output. Dry runs are incredibly useful for two situations: Debugging and testing run options. Using turbo filtering and task graphs for building automations. We hope that this will improve visibility into what turbo is doing, speeding up debugging, and make it easier to leverage turbo in dynamic CI/CD systems. Improved Internal Scheduler and Graph When using turbo run, every package.json task is added to an internal graph to map dependencies based on the inferred relationships defined in your Turborepo pipeline. This task graph allows Turborepo to efficiently schedule incremental concurrent task running and cache task outputs for later use. We have made major improvements to the internal task scheduler and resulting graph structure, resulting in better performance and a better developer experience. For example, in many cases, you will no longer need to use --include-dependencies. Instead, after specifying your task entry points, the new and improved graph will automatically handle this graph resolution on your behalf. Cache Outputs Integrity and Signature Verification You can now configure Turborepo to sign remote cache outputs using HMAC-SHA256 with a secret key before uploading them to the Remote Cache. When Turborepo downloads signed cache artifacts, it will now verify the artifact's integrity and authenticity. Any artifact that fails to verify will be ignored, discarded, and treated as a cache miss by Turborepo. To enable this feature, set the remoteCache options in your turbo.json config file to include signature: true. Then specify your secret key by declaring the TURBO_REMOTE_CACHE_SIGNATURE_KEY environment variable. { \"$schema\": \"[https://turbo.build/schema.json](https://turbo.build/schema.json)\", \"remoteCache\": { // Indicates if signature verification is enabled. \"signature\": true } } Other bug fixes and improvements --sso-team flag now enables teams with SAML tokens to log in through turbo login with correct team permissions. --log-output flag allows you to control what logs are printed to the terminal, and when, allowing you to focus only on what's new. FORCE_COLOR environment variable is now supported. TURBO_FORCE=true environment variable will now force execution. --remote-only and TURBO_REMOTE_ONLY=true will tell turbo to only use Remote Caching. We now show &gt;&gt;&gt; FULL TURBO when there's at least one task attempted. Yarn v2+ with Plug'n'Play (PnP linker) is supported for the turbo run command, but turbo prune is still not fully supported. Fixed regression with chrome tracing if --profile is specified. You can now set concurrency by percentage of CPUs with --concurrency=50% We're hiring! The Turborepo team at Vercel (opens in a new tab) is hiring! We're up to five core team members already this year and are looking to hire even more. We're specifically looking for full-time Senior Build Systems Engineers (opens in a new tab). What's next? Along with seamless incremental adoption/migration and speeding up CI/CD, we've been focusing on improving Turborepo's day-to-day ergonomics, security, and observability. The new --filter flag, signed artifacts, and dry runs are important steps toward those goals. Next up, we'll be focusing an enhanced local development experience, codebase automations, and overall CLI performance. Thank you, contributors Turborepo is the result of the combined work of over 95 individual developers and our core team. This release was brought to you by the contributions of: @gsoltis09, @jaredpalmer, @gaspar09, @shuding, @rajatkulkarni95, @VanTanev, @Kikobeats, @tknickman, @thebanjomatic, @chelkyl, @elado, @finn-orsini, @becca, @weyert, @ekosz.",
  //     raw_url: "/server/pages/blog/turbo-1-2-0.html",
  //     excerpt:
  //       "Yarn v2+ with Plug'n'Play (PnP linker) is supported for the turbo run command, but turbo <mark>prune</mark> is still not fully supported. Fixed regression with chrome tracing if --profile is specified.",
  //     sub_results: [
  //       {
  //         title: "Turborepo 1.2",
  //         url: "/_next/static/chunks/server/pages/blog/turbo-1-2-0.html",
  //         weighted_locations: [
  //           {
  //             weight: 0.16666666666666666,
  //             balanced_score: 33.324566,
  //             location: 890,
  //           },
  //         ],
  //         locations: [890],
  //         excerpt:
  //           "Yarn v2+ with Plug'n'Play (PnP linker) is supported for the turbo run command, but turbo <mark>prune</mark> is still not fully supported. Fixed regression with chrome tracing if --profile is specified.",
  //       },
  //     ],
  //   },
  // },
].map((elem) => ({
  ...elem,
  data: () =>
    new Promise((resolvee) => {
      resolvee(elem.data);
    }),
}));
