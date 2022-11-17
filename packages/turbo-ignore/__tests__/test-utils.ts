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
  logs: Array<string | (() => boolean)>,
  mockConsole: SpyConsole["log"] | SpyConsole["error"]
) {
  logs.forEach((log, idx) => {
    expect(mockConsole).toHaveBeenNthCalledWith(
      idx + 1,
      "≫  ",
      typeof log === "function" ? log() : log
    );
  });
}

type SpyConsole = { log?: any; error?: any };

export function spyConsole({ quiet = true } = {}) {
  let spy: SpyConsole = {};

  beforeEach(() => {
    spy.log = jest
      .spyOn(console, "log")
      .mockImplementation((...args) => !quiet && console.warn(...args));
    spy.error = jest
      .spyOn(console, "error")
      .mockImplementation((...args) => !quiet && console.warn(...args));
  });

  afterEach(() => {
    spy.log.mockClear();
    spy.error.mockClear();
  });

  afterAll(() => {
    spy.log.mockRestore();
    spy.error.mockRestore();
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
