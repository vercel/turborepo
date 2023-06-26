export type TransformErrorOptions = {
  transform?: string;
  fatal?: boolean;
};

export class TransformError extends Error {
  public transform: string;
  public fatal: boolean;

  constructor(message: string, opts?: TransformErrorOptions) {
    super(message);
    this.name = "TransformError";
    this.transform = opts?.transform ?? "unknown";
    this.fatal = opts?.fatal ?? true;
    Error.captureStackTrace(this, TransformError);
  }
}
