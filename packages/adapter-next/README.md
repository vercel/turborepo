# `@turborepo/adaper-next`

An adapter to make Next.js work with Turborepo. For now, this is a pass thru to [`next-transpile-modules`](https://github.com/martpie/next-transpile-modules). In the future, this package will automate the need to specify dependencies, but for now, this is a manual task.

## Install

```shell
yarn add @turborepo/adapter-next --dev
```

## Usage

```js
// next.config.js

module.exports = require('@turborepo/adapter-next')([
  // a list of packages that are used dependencies
  '@sample/dep',
  '@sample/dep-react',
  '@sample/react-internal'
])()
```
