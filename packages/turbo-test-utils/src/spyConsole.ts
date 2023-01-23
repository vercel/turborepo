export type SpyConsole = { log?: any; error?: any; warn?: any };

export default function spyConsole() {
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
