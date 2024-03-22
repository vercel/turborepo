# `@turbo/telemetry`

**NOTE**:
This package is a direct port of the [turbo-telemetry](https://github.com/vercel/turbo/blob/main/crates/turborepo-telemetry) crate.
Any changes made here should also be made to that crate as well.

## Overview

This package provides a way to optionally record anonymous usage data that originates from the turborepo node packages.
This information is used to shape the Turborepo roadmap and prioritize features. You can learn more, including how to opt-out if you'd not like to participate in this anonymous program, by visiting the [documentation](https://turbo.build/repo/docs/telemetry):

## Events

All recorded events can be found by browsing the [event methods on the client class](./src/client.ts).
