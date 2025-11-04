---
title: eslint-config-turbo
description: Learn more about eslint-config-turbo.
---

import { PackageManagerTabs, Tab } from '#components/tabs';

[The `eslint-config-turbo` package](https://www.npmjs.com/package/eslint-config-turbo) helps you find environment variables that are used in your code that are not a part of Turborepo's hashing. Environment variables used in your source code that are not accounted for in `turbo.json` will be highlighted in your editor and errors will show as ESLint output.

## Installation

Install `eslint-config-turbo` into the location where your ESLint configuration is held:

<PackageManagerTabs>

  <Tab value="pnpm">

    ```bash title="Terminal"
    pnpm add eslint-config-turbo --filter=@repo/eslint-config
    ```

  </Tab>

  <Tab value="yarn">

    ```bash title="Terminal"
    yarn workspace @acme/eslint-config add eslint-config-turbo --dev
    ```

  </Tab>

  <Tab value="npm">

    ```bash title="Terminal"
    npm install --save-dev eslint-config-turbo -w @acme/eslint-config
    ```

  </Tab>

  <Tab value="bun (Beta)">

    ```bash title="Terminal"
    bun install --dev eslint-config-turbo --filter=@acme/eslint-config
    ```

  </Tab>
</PackageManagerTabs>

## Usage (Flat Config `eslint.config.js`)

```js title="./packages/eslint-config/base.js"
import turboConfig from 'eslint-config-turbo/flat';

export default [
  ...turboConfig,
  // Other configuration
];
```

You can also configure rules available in the configuration:

```js title="./packages/eslint-config/base.js"
import turboConfig from 'eslint-config-turbo/flat';

export default [
  ...turboConfig,
  // Other configuration
  {
    rules: {
      'turbo/no-undeclared-env-vars': [
        'error',
        {
          allowList: ['^ENV_[A-Z]+$'],
        },
      ],
    },
  },
];
```

## Usage (Legacy `eslintrc*`)

Add `turbo` to the extends section of your eslint configuration file. You can omit the `eslint-config-` prefix:

```json title="./packages/eslint-config/base.json"
{
  "extends": ["turbo"]
}
```

You can also configure rules available in the configuration:

```json title="./packages/eslint-config/base.json"
{
  "plugins": ["turbo"],
  "rules": {
    "turbo/no-undeclared-env-vars": [
      "error",
      {
        "allowList": ["^ENV_[A-Z]+$"]
      }
    ]
  }
}
```
