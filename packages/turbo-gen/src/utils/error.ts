export type GenerateErrorType =
  // custom errors
  | "plop_error_running_generator"
  | "plop_unable_to_load_config"
  | "plop_generator_not_found"
  | "plop_no_config"
  | "config_directory_already_exists"
  // default
  | "unknown";

export interface GeneratorErrorOptions {
  type?: GenerateErrorType;
}

export class GeneratorError extends Error {
  public type: GenerateErrorType;

  constructor(message: string, opts?: GeneratorErrorOptions) {
    super(message);
    this.name = "GenerateError";
    this.type = opts?.type ?? "unknown";
    Error.captureStackTrace(this, GeneratorError);
  }
}
