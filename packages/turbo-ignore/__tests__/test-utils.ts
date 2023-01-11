export function mockEnv() {
  const OLD_ENV = process.env;

  beforeEach(() => {
    jest.resetModules();
    process.env = { ...OLD_ENV };
  });

  afterAll(() => {
    process.env = OLD_ENV;
  });
}

export function validateLogs(
  logs: Array<string | (() => boolean | Array<any>)>,
  mockConsole: SpyConsole["log"] | SpyConsole["error"]
) {
  logs.forEach((log, idx) => {
    if (typeof log === "function") {
      const expected = log();
      expect(mockConsole).toHaveBeenNthCalledWith(
        idx + 1,
        ...(Array.isArray(expected) ? expected : [expected])
      );
    } else {
      expect(mockConsole).toHaveBeenNthCalledWith(idx + 1, "≫  ", log);
    }
  });
}

type SpyConsole = { log?: any; error?: any; warn?: any };

export function spyConsole() {
  let spy: SpyConsole = {};

  beforeEach(() => {
    spy.log = jest.spyOn(console, "log").mockImplementation(() => {});
    spy.error = jest.spyOn(console, "error").mockImplementation(() => {});
    spy.warn = jest.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    spy.log.mockClear();
    spy.error.mockClear();
    spy.warn.mockClear();
  });

  afterAll(() => {
    spy.log.mockRestore();
    spy.error.mockRestore();
    spy.warn.mockRestore();
  });

  return spy;
}

export type SpyExit = { exit?: any };

export function spyExit() {
  let spy: SpyExit = {};

  beforeEach(() => {
    spy.exit = jest
      .spyOn(process, "exit")
      .mockImplementation(() => undefined as never);
  });

  afterEach(() => {
    spy.exit.mockClear();
  });

  afterAll(() => {
    spy.exit.mockRestore();
  });

  return spy;
}
