# `eslint-plugin-turbo`

Ease configuration for Turborepo

## Installation

1. You'll first need to install [ESLint](https://eslint.org/):

```sh
npm install eslint --save-dev
```

2. Next, install `eslint-plugin-turbo`:

```sh
npm install eslint-plugin-turbo --save-dev
```

## Usage

Add `turbo` to the plugins section of your `.eslintrc` configuration file. You can omit the `eslint-plugin-` prefix:

```json
{
  "plugins": ["turbo"]
}
```

Then configure the rules you want to use under the rules section.

```json
{
  "rules": {
    "turbo/no-undeclared-env-vars": "error"
  }
}
```

### Example

```json
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
