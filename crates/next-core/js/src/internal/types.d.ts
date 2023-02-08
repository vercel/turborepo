export type IpcOutgoingMessage =
  | {
      type: "response";
      statusCode: number;
      headers: Array<[string, string]>;
      body: string;
    }
  | { type: "rewrite"; path: string };
