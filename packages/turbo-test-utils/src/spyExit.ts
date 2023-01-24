export type SpyExit = { exit?: any };

export default function spyExit() {
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
