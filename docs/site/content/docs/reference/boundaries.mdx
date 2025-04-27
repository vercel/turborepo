---
title: boundaries
description: API reference for the `turbo boundaries` command
---

import { ExperimentalBadge } from '#components/experimental-badge';
import { Callout } from '#components/callout';

<ExperimentalBadge>Experimental</ExperimentalBadge>

Boundaries ensure that Turborepo features work correctly by checking for package manager Workspace violations.

```bash title="Terminal"
turbo boundaries
```

<Callout title="Boundaries RFC">
  This feature is experimental, and we're looking for your feedback on [the
  Boundaries RFC](https://github.com/vercel/turborepo/discussions/9435).
</Callout>

This command will notify for two types of violations:

- Importing a file outside of the package's directory
- Importing a package that is not specified as a dependency in the package's `package.json`

## Tags

Boundaries also has a feature that lets you add tags to packages. These tags can be used to create rules
for Boundaries to check. For example, you can add an `internal` tag to your UI package:

```json title="./packages/ui/turbo.json"
{
  "tags": ["internal"]
}
```

And then declare a rule that packages with a `public` tag cannot depend on packages with an `internal` tag:

```json title="./turbo.json"
{
  "boundaries": {
    "tags": {
      "public": {
        "dependencies": {
          "deny": ["internal"]
        }
      }
    }
  }
}
```

Alternatively, you may want `public` packages to only depend on other `public` packages:

```json title="turbo.json"
{
  "boundaries": {
    "tags": {
      "public": {
        "dependencies": {
          "allow": ["public"]
        }
      }
    }
  }
}
```

Likewise, you can add restrictions for a tag's dependents, i.e. packages that import packages with the tag.

```json title="turbo.json"
{
  "boundaries": {
    "tags": {
      "private": {
        "dependents": {
          "deny": ["public"]
        }
      }
    }
  }
}
```

Package names can also be used in place of a tag in allow and deny lists.

```json title="turbo.json"
{
  "boundaries": {
    "tags": {
      "private": {
        "dependents": {
          "deny": ["@repo/my-pkg"]
        }
      }
    }
  }
}
```

Tags allow you to ensure that the wrong package isn't getting imported somewhere in your graph. These rules are
applied even for dependencies of dependencies, so if you import a package that in turn imports another package
with a denied tag, you will still get a rule violation.
