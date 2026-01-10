import { createOpenAPI } from "fumadocs-openapi/server";
import spec from "./remote-cache-openapi.json";

export const openapi = createOpenAPI({
  // @ts-expect-error - fumadocs-openapi types are out of sync with runtime
  input: () => ({
    "remote-cache": spec
  })
});
