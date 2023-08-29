type Spy = jest.SpyInstance | undefined;

export interface SpyExit {
  exit: Spy;
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
