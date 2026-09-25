The `web` build reads the repository-root `build-settings.json` to produce its page. Editing the title sometimes leaves a cached page with the old title. An unrelated `@repo/ui` build doesn't use that file and should keep its cache hit.

Fix the build configuration so changes to that root file invalidate `web`'s build, but not unrelated workspace builds. Keep normal source files and generated outputs accounted for.
