---
title: Travis CI
description: How to use Travis CI with Turborepo to optimize your CI workflow
---

import { PackageManagerTabs, Tab } from '#components/tabs';

The following example shows how to use Turborepo with [Travis CI](https://www.travis-ci.com/).

For a given root `package.json`:

```json title="./package.json"
{
  "name": "my-turborepo",
  "scripts": {
    "build": "turbo run build",
    "test": "turbo run test"
  },
  "devDependencies": {
    "turbo": "latest"
  }
}
```

And a `turbo.json`:

```json title="./turbo.json"
{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {
    "build": {
      "outputs": [".svelte-kit/**"],
      "dependsOn": ["^build"]
    },
    "test": {
      "dependsOn": ["^build"]
    }
  }
}
```

Create a file called `.travis.yml` in your repository with the following contents:

<PackageManagerTabs>
    <Tab value="pnpm">

      ```yaml title=".travis.yml"
      language: node_js
      node_js:
        - lts/*
      cache:
        npm: false
        directories:
          - "~/.pnpm-store"
      before_install:
        - curl -f https://get.pnpm.io/v6.16.js | node - add --global pnpm@6.32.2
        - pnpm config set store-dir ~/.pnpm-store
      install:
        - pnpm install
      script:
        - pnpm build
      script:
        - pnpm test
      ```

      > For more information visit the pnpm documentation section on Travis CI integration, view it [here](https://pnpm.io/continuous-integration#travis)

  </Tab>
    <Tab value="yarn">
      Travis CI detects the use of Yarn by the presence of `yarn.lock`. It will automatically ensure it is installed.

      ```yaml title=".travis.yml"
      language: node_js
      node_js:
        - lts/*
      install:
        - yarn
      script:
        - yarn build
      script:
        - yarn test
      ```

  </Tab>

<Tab value="npm">

    ```yaml title=".travis.yml"
    language: node_js
    node_js:
      - lts/*
    install:
      - npm install
    script:
      - npm run build
    script:
      - npm run test
    ```

  </Tab>

    <Tab value="bun (Beta)">

      ```yaml title=".travis.yml"
      language: node_js
      node_js:
        - lts/*
      cache:
        npm: false
        directories:
          - "~/.pnpm-store"
      before_install:
        - curl -fsSL https://bun.sh/install | bash
      install:
        - bun install
      script:
        - bun run build
      script:
        - bun run test
      ```

  </Tab>
</PackageManagerTabs>

## Remote Caching

To use Remote Caching, retrieve the team and token for the Remote Cache for your provider. In this example, we'll use [Vercel Remote Cache](https://vercel.com/docs/monorepos/remote-caching):

- `TURBO_TOKEN` - The Bearer token to access the Remote Cache
- `TURBO_TEAM` - The slug of the Vercel team to share the artifacts with

To use Vercel Remote Caching, you can get the value of these variables in a few steps:

1. Create a Scoped Access Token to your account in the [Vercel Dashboard](https://vercel.com/account/tokens)

![Vercel Access Tokens](/images/docs/vercel-create-token.png)

Copy the value to a safe place. You'll need it in a moment.

2. Go to your Travis repository settings and scroll down to the _Environment Variables_ section. Create a new variable called `TURBO_TOKEN` and enter the value of your Scoped Access Token.

![Travis CI Variables](/images/docs/travis-ci-environment-variables.png)

3. Make a second secret called `TURBO_TEAM` and set it to your team slug - the part after `vercel.com/` in [your Team URL](https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fsettings&title=Find+Team+URL). For example, the slug for `vercel.com/acme` is `acme`. 

4. Travis CI automatically loads environment variables stored in project settings into the CI environment. No modifications are necessary for the CI file.
