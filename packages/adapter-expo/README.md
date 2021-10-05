# `@turborepo/adapter-expo`

An adapter to make [Expo](https://expo.io) work with Turborepo. For now it is a pass thru to [`expo-yarn-workspaces`](https://www.npmjs.com/package/expo-yarn-workspaces). It finesses Yarn workspaces, Metro, Turborepo, and the Expo repository to work together.

**Note:** This package runs only on macOS and Linux.

## How apps work with workspaces

Each Expo app in the repository that is intended to work with Yarn workspaces (as opposed to being tested in a non-workspace environment) does the steps described below. All of the steps are important and need to be followed carefully.

## Install

### Add `@turborepo/adapter-expo` as a devDependency

```shell
yarn add @turborepo/adapter-export --dev
```

This adds scripts provided by `@turborepo/adapter-expo` to the project under its `node_modules/.bin` directory and also defines modules the app will use.

### Add a `postinstall` script to package.json

**Add `"postinstall": "turborepo-adapter-expo postinstall"` under the `"scripts"` object in the app's package.json file.** The postinstall script does two things:

1. It creates symlinks for packages that some programs expect to exist under `node_modules`, namely `expo` and `react-native`. These symlinks point to the respective packages installed in the workspace root.

2. It generates an entry module for the app that assumes your app's root component is exported from `App.js` (`App.${platform}.js` also works). This is similar to conventional Expo apps, but we need to generate a different entry module because Metro does not use the logical path to the entry module within the symlinked `expo` package.

### Define the entry module in the `"main"` field of package.json

The postinstall script determines the location of the generated entry module by looking at the `"main"` field in package.json. In a conventional Expo app, the value of the `"main"` field is `node_modules/expo/AppEntry.js`. In a workspace in the Expo repo, **specify `"__generated__/AppEntry.js"` as the value of the `"main"` field in package.json.**

You can specify other paths too. The `.expo` directory is convenient since it already contains auto-generated files and is .gitignore'd.

### Create a file named `metro.config.js` and reference it from app.json

**Create a file named `metro.config.js` in the app's base directory with these contents:**

```js
const { createMetroConfiguration } = require('@turborepo/adapter-expo')

module.exports = createMetroConfiguration(__dirname)
```

The `@turborepo/adapter-expo` package defines a Metro configuration object that makes Metro work with Yarn workspaces in the Expo repo. It configures Metro to include packages from the workspace root, resolves symlinked packages, excludes modules from Haste's module system, and exclude modules in the native Android and Xcode projects. You can further customize this configuration object before exporting it, if needed.

**Aside:** when starting the project, run `expo start --clear` so Metro uses the latest configuration instead of working with cached values.
