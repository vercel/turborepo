import { expect } from "@jest/globals";
import type { SpyConsole } from "./spyConsole";

type Matcher = ReturnType<typeof expect.stringContaining>;

export function validateLogs(
  spy: SpyConsole[keyof SpyConsole],
  args: Array<Array<string | Matcher>>
) {
  args.forEach((arg, idx) => {
    expect(spy).toHaveBeenNthCalledWith(idx + 1, ...arg);
  });
}
