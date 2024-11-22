# `eslint-config-turbo`

Ease configuration for Turborepo

## Installation

1. You'll first need to install [ESLint](https://eslint.org/):

```sh
npm install eslint --save-dev
```

2. Next, install `eslint-config-turbo`:

```sh
npm install eslint-config-turbo --save-dev
```

## Usage (Flat Config `eslint.config.js`)

```
{
import turboConfig from "eslint-config-turbo/flat";

export default [
  ...turboConfig
  // Other configuration
];
}
```

## Usage (Legacy `eslintrc*`)

Add `turbo` to the extends section of your eslint configuration file. You can omit the `eslint-config-` prefix:

```json
{
  "extends": ["turbo"]
}
```
