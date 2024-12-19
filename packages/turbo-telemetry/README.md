# `@turbo/telemetry`

**NOTE**:
This package is a direct port of the [turbo-telemetry](https://github.com/vercel/turborepo/blob/main/crates/turborepo-telemetry) crate.
Any changes made here should also be made to that crate as well.

## Overview

This package provides a way to optionally record anonymous usage data that originates from the turborepo node packages.
This information is used to shape the Turborepo roadmap and prioritize features. You can learn more, including how to opt-out if you'd not like to participate in this anonymous program, by visiting the [documentation](https://turbo.build/repo/docs/telemetry):

## Events

Each package must create a subclass of the main telemetry client and implement specific methods for each telemetry event. All recorded events can be found by browsing the [packages classes](./src/events).

## Usage

1. Init the client with your package name and version:

```ts
import { initTelemetry } from "@turbo/telemetry";
import pkgJson from "../package.json";

const { telemetry } = await initTelemetry({
  name: pkgJson.name,
  version: pkgJson.version,
});
```

2. Send events

```ts
telemetry.myCustomEventName({
  // event properties
});
```

3. Close the client before exiting

```ts
await telemetry.close();
```
