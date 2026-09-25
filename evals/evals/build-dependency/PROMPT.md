Our `web` workspace reads the built artifact from `@repo/ui`. On a clean checkout, a root build is unreliable: `web` can start before `ui` has produced its artifact.

Fix the monorepo's task setup so building `web` schedules the library build before it, while preserving caching of the build outputs. Don't hardcode the result into the application or serialize unrelated workspaces.
