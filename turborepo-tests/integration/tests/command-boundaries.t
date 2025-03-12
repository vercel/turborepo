Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh boundaries

Ignore all errors
  $ ${TURBO} boundaries --ignore=all
  Checking packages...
  patching apps(\\|/)my-app(\\|/)(index|types).ts (re)
  patching apps(\\|/)my-app(\\|/)(index|types).ts (re)
  [1]

  $ git diff
  diff --git a/apps/my-app/index.ts b/apps/my-app/index.ts
  index 6baec29..d4c7af6 100644
  --- a/apps/my-app/index.ts
  +++ b/apps/my-app/index.ts
  @@ -1,9 +1,13 @@
   // Import directly from a file instead of via package name
  +// @boundaries-ignore automatically added by `turbo boundaries --ignore=all`
   import { blackbeard } from "../../packages/another/index.jsx";
   // Import type without "type" specifier in import
  +// @boundaries-ignore automatically added by `turbo boundaries --ignore=all`
   import { Ship } from "ship";
  +// @boundaries-ignore automatically added by `turbo boundaries --ignore=all`
   import { Ship } from "@types/ship";
   // Import package that is not specified
  +// @boundaries-ignore automatically added by `turbo boundaries --ignore=all`
   import { walkThePlank } from "module-package";
   
   // Import from a package that is not specified, but we have `@boundaries-ignore` on it
  diff --git a/apps/my-app/types.ts b/apps/my-app/types.ts
  index ce28692..3615d9c 100644
  --- a/apps/my-app/types.ts
  +++ b/apps/my-app/types.ts
  @@ -1,4 +1,6 @@
  +// @boundaries-ignore automatically added by `turbo boundaries --ignore=all`
   import { blackbeard } from "@/../../packages/another/index.jsx";
  +// @boundaries-ignore automatically added by `turbo boundaries --ignore=all`
   import { blackbead } from "!";
   
   export interface Pirate {

