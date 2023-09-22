export type Spy = jest.SpyInstance | undefined;

export interface SpyConsole {
  log: Spy;
  error: Spy;
  warn: Spy;
}

export function spyConsole() {
  const spy: SpyConsole = {
    log: undefined,
    error: undefined,
    warn: undefined,
  };

  beforeEach(() => {
    spy.log = jest.spyOn(console, "log").mockImplementation(() => {
      // do nothing
    });
    spy.error = jest.spyOn(console, "error").mockImplementation(() => {
      // do nothing
    });
    spy.warn = jest.spyOn(console, "warn").mockImplementation(() => {
      // do nothing
    });
  });

  afterEach(() => {
    spy.log?.mockClear();
    spy.error?.mockClear();
    spy.warn?.mockClear();
  });

  afterAll(() => {
    spy.log?.mockRestore();
    spy.error?.mockRestore();
    spy.warn?.mockRestore();
  });

  return spy;
}
