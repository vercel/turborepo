export type ConvertErrorType =
  // package manager general
  | "package_manager-unexpected"
  | "package_manager-already_in_use"
  | "package_manager-unable_to_detect"
  | "package_manager-unsupported_version"
  // package manager specific
  | "pnpm-workspace_parse_error"
  // package.json
  | "package_json-parse_error"
  | "package_json-missing"
  // other
  | "invalid_directory"
  | "error_removing_node_modules"
  // default
  | "unknown";

export type ConvertErrorOptions = {
  type?: ConvertErrorType;
};

export class ConvertError extends Error {
  public type: ConvertErrorType;

  constructor(message: string, opts?: ConvertErrorOptions) {
    super(message);
    this.name = "ConvertError";
    this.type = opts?.type ?? "unknown";
    Error.captureStackTrace(this, ConvertError);
  }
}
