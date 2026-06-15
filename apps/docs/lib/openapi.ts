import { createOpenAPI, type OpenAPIOptions } from "fumadocs-openapi/server";
import spec from "./remote-cache-openapi.json";

type OpenAPIInput = Exclude<OpenAPIOptions["input"], string[] | undefined>;

export const openapi = createOpenAPI({
  input: {
    "remote-cache": spec as OpenAPIInput["remote-cache"]
  } satisfies OpenAPIInput
});
