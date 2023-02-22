export class ConvertError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ConvertError";
    Error.captureStackTrace(this, ConvertError);
  }
}
