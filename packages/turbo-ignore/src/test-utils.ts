export function spyConsole() {
  let spy: { log?: any; error?: any } = {};

  beforeEach(() => {
    spy.log = jest.spyOn(console, "log").mockImplementation(() => {});
    spy.error = jest.spyOn(console, "error").mockImplementation(() => {});
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

export function spyExit() {
  let spy: { exit?: any } = {};

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
