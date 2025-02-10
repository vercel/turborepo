// Import directly from a file instead of via package name
import { blackbeard } from "../../packages/another/index.jsx";
// Import type without "type" specifier in import
import { Ship } from "ship";
import { Ship } from "@types/ship";
// Import package that is not specified
import { walkThePlank } from "module-package";

// Import from a package that is not specified, but we have `@boundaries-ignore` on it
// @boundaries-ignore
import { walkThePlank } from "module-package";

// Import also works with other ignore directives
// @boundaries-ignore
// @ts-ignore
import { walkThePlank } from "module-package";

// import also works with whitespace
//                      @boundaries-ignore
import { walkThePlank } from "module-package";

// @boundaries-ignore

import { walkThePlank } from "module-package";
