# Ensure all environment variables are correctly included in cache keys (`no-undeclared-env-vars`)

Ensures that all detectable usage of environment variables are correctly included in cache keys. This ensures build outputs remain correctly cacheable across environments.

## Rule Details

This rule aims to prevent users from forgetting to include an environment variable in their `turbo.json` configuration.

The following examples assume the following code:

```js
const client = MyAPI({ token: process.env.MY_API_TOKEN });
```

Examples of **incorrect** code for this rule:

```json
{
  "pipeline": {
    "build": {
      "dependsOn": ["^build"],
      "outputs": ["dist/**", ".next/**"]
    },
    "lint": {
      "outputs": []
    },
    "dev": {
      "cache": false
    }
  }
}
```

Examples of **correct** code for this rule:

```json
{
  "globalDependencies": ["$MY_API_TOKEN"]
  "pipeline": {
    "build": {
      "dependsOn": ["^build"],
      "outputs": ["dist/**", ".next/**"]
    },
    "lint": {
      "outputs": []
    },
    "dev": {
      "cache": false
    }
  }
}
```

```json
{
  "pipeline": {
    "build": {
      "dependsOn": ["^build", "$MY_API_TOKEN"],
      "outputs": ["dist/**", ".next/**"]
    },
    "lint": {
      "outputs": []
    },
    "dev": {
      "cache": false
    }
  }
}
```

## Options

| Option        | Required | Default     | Details                                                                                                                                     | Example                                      |
| ------------- | -------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| `turboConfig` | No       | Auto-detect | Resolved `turbo.json` configuration                                                                                                         | `require("./turbo.json")`                    |
| `allowList`   | No       | []          | An array of strings (or regular expressions) to exclude. NOTE: an env variable should only be excluded if it has no effect on build outputs | `["MY_API_TOKEN", "^MY_ENV_PREFIX_[A-Z]+$"]` |

## Further Reading

- [Alter Caching Based on Environment Variables and Files](https://turbo.build/repo/docs/core-concepts/caching#alter-caching-based-on-environment-variables-and-files)
