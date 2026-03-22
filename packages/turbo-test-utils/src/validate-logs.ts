import { expect } from "@jest/globals";
import type { SpyConsole } from "./spy-console";

type Matcher = ReturnType<typeof expect.stringContaining>;

export function validateLogs(
  spy: SpyConsole[keyof SpyConsole],
  args: Array<Array<string | Matcher>>
) {
  for (const [idx, arg] of args.entries()) {
    expect(spy).toHaveBeenNthCalledWith(idx + 1, ...arg);
  }
}
