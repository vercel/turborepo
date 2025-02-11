// Import directly from a file instead of via package name
import { blackbeard } from "../../packages/another/index.jsx";
// Import type without "type" specifier in import
import { Ship } from "ship";
import { Ship } from "@types/ship";
// Import package that is not specified
import { walkThePlank } from "module-package";

// Import from a package that is not specified, but we have `@boundaries-ignore` on it
// @boundaries-ignore this is a test
import { walkThePlank } from "module-package";

// Import also works with other ignore directives
// @boundaries-ignore this is a test
// @ts-ignore
import { walkThePlank } from "module-package";

// import also works with whitespace
//                      @boundaries-ignore here's another reason
import { walkThePlank } from "module-package";

// @boundaries-ignore one more reason

import { walkThePlank } from "module-package";
