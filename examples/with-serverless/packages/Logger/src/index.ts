import {
  Logger as LamdaLogger,
  LogFormatter,
} from "@aws-lambda-powertools/logger";

export interface LoggerOptions {
  serviceName: string;
}

export class Logger {
  logger: LamdaLogger;

  constructor(param: LoggerOptions) {
    this.logger = new LamdaLogger(param);
  }

  info(message: string): void {
    this.logger.info({
      message,
    });
  }

  warn(message: string): void {
    this.logger.warn({
      message,
    });
  }

  error(message: string, error: Error): void {
    this.logger.error({
      message,
      error,
    });
  }
};
