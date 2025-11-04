---
title: Buildkite
description: Learn how to use Buildkite with Turborepo.
---

import { PackageManagerTabs, Tab } from '#components/tabs';

The following example shows how to use Turborepo with [Buildkite](https://buildkite.com/).

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
      "outputs": [".next/**", "!.next/cache/**"],
      "dependsOn": ["^build"]
    },
    "test": {
      "dependsOn": ["^build"]
    }
  }
}
```

Create a file called `.buildkite/pipeline.yml` in your repository with the following contents:

<PackageManagerTabs>

  <Tab value="pnpm">

    ```yaml title=".buildkite/pipeline.yml"
    steps:
      - label: ":test_tube: Test"
        command: |
          pnpm install
          pnpm test

      - label: ":hammer: Build"
        command: |
          pnpm install
          pnpm build
    ```

  </Tab>
  <Tab value="yarn">

    ```yaml title=".buildkite/pipeline.yml"
    steps:
      - label: ":test_tube: Test"
        command: |
          yarn
          yarn test

      - label: ":hammer: Build"
        command: |
          yarn
          yarn build
    ```

  </Tab>

<Tab value="npm">

    ```yaml title=".buildkite/pipeline.yml"
    steps:
      - label: ":test_tube: Test"
        command: |
          npm install
          npm test

      - label: ":hammer: Build"
        command: |
          npm install
          npm run build
    ```

  </Tab>

  <Tab value="bun (Beta)">

    ```yaml title=".buildkite/pipeline.yml"
    steps:
      - label: ":test_tube: Test"
        command: |
          bun install
          bun run test

      - label: ":hammer: Build"
        command: |
          bun install
          bun run build
    ```

  </Tab>
</PackageManagerTabs>

## Create a Pipeline

To create your pipeline in the Buildkite dashboard, you'll need to first upload the pipeline definition from your repository.

1. Select **Pipelines** to navigate to the Buildkite dashboard.

2. Select **New pipeline**.

3. Enter your pipeline's details in the respective **Name** and **Description** fields.

4. In the **Steps** editor, ensure there's a step to upload the definition from your repository:

```yaml title=".buildkite/pipeline.yml"
steps:
  - label: ':pipeline:'
    command: buildkite-agent pipeline upload
```

5. Select **Create Pipeline**, then click **New Build**, then select **Create Build**.

Run the pipeline whenever you make changes you want to verify.

## Remote Caching

To use Remote Caching, retrieve the team and token for the Remote Cache for your provider. In this example, we'll use [Vercel Remote Cache](https://vercel.com/docs/monorepos/remote-caching):

- `TURBO_TOKEN` - The Bearer token to access the Remote Cache
- `TURBO_TEAM` - The account to which the monorepo belongs

To use Vercel Remote Caching, you can get the value of these variables in a few steps:

1. Create a Scoped Access Token to your account in the [Vercel Dashboard](https://vercel.com/account/tokens). Copy the value to a safe place. You'll need it in a moment.

   ![Vercel Access Tokens](/images/docs/vercel-create-token.png)

2. Obtain [your Team URL](https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fsettings&title=Find+Team+URL) and copy its value as well. Both values will be used in the next step.

3. In the Buildkite dashboard, create two new [Buildkite secrets](https://buildkite.com/docs/pipelines/security/secrets/buildkite-secrets), one for each value. Name them `TURBO_TOKEN` and `TURBO_TEAM`.

4. Update `pipeline.yml` to fetch and apply `TURBO_TOKEN` and `TURBO_TEAM` as environment variables with the [Buildkite Secrets](https://github.com/buildkite-plugins/secrets-buildkite-plugin) plugin as shown. (For additional secret-management options, read [Managing pipeline secrets](https://buildkite.com/docs/pipelines/security/secrets/managing) in the Buildkite documentation.)

   ```yaml title=".buildkite/pipeline.yml"
   steps:
     - label: ':test_tube: Test'
       command: |
         npm install
         npm test
       plugins:
         - secrets:
             variables:
               TURBO_TOKEN: TURBO_TOKEN
               TURBO_TEAM: TURBO_TEAM

     - label: ':hammer: Build'
       command: |
         npm install
         npm run build
       plugins:
         - secrets:
             variables:
               TURBO_TOKEN: TURBO_TOKEN
               TURBO_TEAM: TURBO_TEAM
   ```

   Commit and push these changes to your repository, and on the next pipeline run, the secrets will be applied and Vercel Remote Caching will be active.
