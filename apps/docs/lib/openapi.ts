import { createOpenAPI } from "fumadocs-openapi/server";
import spec from "./remote-cache-openapi.json";

export const openapi = createOpenAPI({
  input: () => ({
    // OpenAPI spec is validated at build time; cast for fumadocs-openapi 10.10 schema typing.
    "remote-cache": spec as never
  })
});
