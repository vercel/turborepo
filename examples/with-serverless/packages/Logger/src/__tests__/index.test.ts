import { Logger } from "..";

describe("lambda logger", () => {
  let logger: Logger;
  beforeAll(() => {
    logger = new Logger({
      serviceName: "test-lambda-function"
    });

  });

  it("test error logger", () => {
    const consoleSpy = jest.spyOn(logger, 'error');
    
    logger.error(
      "test error",
      new Error("test error")
    );

    expect(consoleSpy).toHaveBeenCalledWith("test error", new Error("test error"));
  });

  it("test warn logger", () => {
    const consoleSpy = jest.spyOn(logger, 'warn');
    
    logger.warn(
      "test warning"
    );

    expect(consoleSpy).toHaveBeenCalledWith("test warning");
  });

  it("test info logger", () => {
    const consoleSpy = jest.spyOn(logger, 'info');
    
    logger.info(
      "test information",
    );

    expect(consoleSpy).toHaveBeenCalledWith("test information");
  });
});
