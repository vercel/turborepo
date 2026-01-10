import { createOpenAPI } from "fumadocs-openapi/server";
import spec from "./remote-cache-openapi.json";

export const openapi = createOpenAPI({
  input: () => ({
    "remote-cache": spec
  })
});
