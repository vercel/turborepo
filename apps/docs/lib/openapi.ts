import { createOpenAPI } from "fumadocs-openapi/server";
import spec from "./remote-cache-openapi.json";

type OpenApiInput = NonNullable<
  Parameters<typeof createOpenAPI>[0]
>["input"] extends () => infer R
  ? R
  : never;

export const openapi = createOpenAPI({
  input: () =>
    ({
      "remote-cache": spec
    }) as OpenApiInput
});
