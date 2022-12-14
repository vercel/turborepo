import { StackFrame } from "stacktrace-parser";
export type StructuredError = {
  name: string;
  message: string;
  stack: StackFrame[];
};
export declare function structuredError(e: Error): StructuredError;
export type Ipc<TIncoming, TOutgoing> = {
  recv(): Promise<TIncoming>;
  send(message: TOutgoing): Promise<void>;
  sendError(error: Error): Promise<never>;
};
export declare const IPC: Ipc<unknown, unknown>;
