---
title: Framework bindings in libraries
description: Learn how to create framework bindings in packages.
---

import { PackageManagerTabs, Tab } from '#components/tabs';
import { Callout } from '#components/callout';

Framework bindings in a [Library Package](/docs/core-concepts/package-types#library-packages) integrate your library's code more deeply with a framework by leveraging APIs from the framework directly in the library.

To do this, use the `peerDependencies` field in `package.json` of the library, which makes the framework APIs available in your library without installing it directly in the package.

<Callout type="good-to-know">
  On this page, we'll be using Next.js for examples, but the concepts below
  apply to any framework or other dependency.
</Callout>

## Example

Add a `peerDependency` to your library for the dependency that you intend to create bindings for.

```json title="./packages/ui/package.json"
{
  "name": "@repo/ui",
  "peerDependencies": {
    "next": "*"
  }
}
```

<Callout type="good-to-know">
  In the example above, the `peerDependency` for `next` accepts any version. You
  may want to specify a range (for example, `">=15"`) according to your needs.
  Additionally, for older package managers, you may need to instruct your
  package manager to install peer dependencies with configuration, or add the
  dependency to `devDependencies` as a workaround.
</Callout>

This will make the dependency available in your library, allowing you to write code like below. Note the `className` prop, which sets a default styling for this component in the monorepo and can be overridden in the `props` object.

```tsx title="./packages/ui/src/link.tsx"
import Link from 'next/link';
import type { ComponentProps } from 'react';

type CustomLinkProps = ComponentProps<typeof Link>;

export function CustomLink({ children, ...props }: CustomLinkProps) {
  return (
    <Link className="text-underline hover:text-green-400" {...props}>
      {children}
    </Link>
  );
}
```

The version of `next` that will be resolved for the package will come from the consumers of the library. For example, if Next.js 15 is installed in your applications, the TypeScript types and APIs for `next` will also be Next.js 15.

## Splitting framework bindings using entrypoints

Using export paths to split a package into framework-specific entrypoints is the simplest way to add bindings to a library that aims to support multiple frameworks. By splitting entrypoints, bundlers have an easier time understanding the framework you intend to target and you're less likely to see strange bundling errors.

The example below shows a library with two entrypoints, each for a different type of link component. These abstractions likely contain your own styles, APIs, and other adjustments on top of the element they're wrapping.

- `./link`: An `<a>` HTML tag with some default styles from your design system
- `./next-js/link`: A customized version of [the Next.js `Link` component](https://nextjs.org/docs/app/building-your-application/routing/linking-and-navigating#link-component) with props that are preset to your organization's preferences
- `./svelte/link`: A customized version of an [`a` tag for Svelte](https://svelte.dev/docs/kit/link-options) with presets.

```json title="./packages/ui/package.json"
{
  "exports": {
    "./link": "./dist/link.js",
    "./next-js/link": "./dist/next-js/link.js"
  },
  "peerDependencies": {
    "next": "*"
  }
}
```

<Callout type="good-to-know">
  In the example above, the `peerDependency` for `next` accepts any version. You
  may want to specify a range (for example, `">=15"`) according to your needs.
</Callout>

This concept can be applied to any number of frameworks or other dependencies that you'd like to provide bindings for.
