# `only-turbo`

This is a utility program which you can use to ensure that a package script was invoked by `turbo`.

It further makes an attempt to identify what command the user was trying to invoke to provide useful feedback.

## Usage

You would typically use this to prefix commands inside of monorepo workspaces to prevent them from being executed without `turbo`.

```json /turbo.json
{
  "pipeline": {
    "build": {
      "dependsOn": ["^build"]
    }
  }
}
```

```json /package.json
{
  "scripts": {
    "build": "turbo run build"
  }
}
```

```json /apps/web/package.json
{
  "scripts": {
    "build": "only-turbo next build"
  }
}
```
