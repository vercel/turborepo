import { afterAll, afterEach, beforeEach, jest } from "@jest/globals";
import type { MockInstance } from "jest-mock";

export interface SpyExit {
  exit: MockInstance<(code?: number) => never> | undefined;
}

export function spyExit() {
  const spy: SpyExit = {
    exit: undefined,
  };

  beforeEach(() => {
    spy.exit = jest
      .spyOn(process, "exit")
      .mockImplementation(() => undefined as never);
  });

  afterEach(() => {
    spy.exit?.mockClear();
  });

  afterAll(() => {
    spy.exit?.mockRestore();
  });

  return spy;
}
