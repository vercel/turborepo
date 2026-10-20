"use client";

import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";

import {
  buildResizeMessage,
  buildStartMessage,
  buildWebSocketUrl,
  parseServerMessage
} from "../lib/sandbox-terminal-protocol";

export function WorkspaceTerminal({
  workspaceId
}: {
  readonly workspaceId: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const terminal = new Terminal({
      cursorBlink: true,
      fontFamily: "var(--font-geist-mono), monospace",
      fontSize: 13,
      lineHeight: 1.35,
      theme: { background: "#000000", foreground: "#ededed" }
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(containerRef.current!);
    fitAddon.fit();
    let socket: WebSocket | undefined;
    let cancelled = false;

    void fetch(`/api/workspaces/${encodeURIComponent(workspaceId)}/terminal`, {
      body: "{}",
      headers: {
        "content-type": "application/json",
        "x-operator-action": "open-workspace-terminal"
      },
      method: "POST"
    })
      .then(async (response) => {
        const body = (await response.json()) as {
          readonly error?: string;
          readonly token?: string;
          readonly url?: string;
        };
        if (!response.ok || !body.url || !body.token)
          throw new Error(
            body.error ?? `Could not open terminal (${response.status}).`
          );
        if (cancelled) return;
        socket = new WebSocket(buildWebSocketUrl(body.url, body.token));
        socket.binaryType = "arraybuffer";
        socket.addEventListener("open", () => {
          const { cols, rows } = fitAddon.proposeDimensions() ?? {
            cols: 80,
            rows: 24
          };
          socket?.send(JSON.stringify(buildStartMessage(cols, rows)));
          terminal.focus();
        });
        socket.addEventListener("message", (event) => {
          const message = parseServerMessage(event.data);
          if (message.kind === "output") terminal.write(message.data);
          if (message.kind === "exit")
            terminal.writeln(
              `\r\nSession closed${message.code === null ? "." : ` with exit code ${message.code}.`}`
            );
        });
      })
      .catch((cause) => {
        if (!cancelled)
          setError(
            cause instanceof Error ? cause.message : "Could not open terminal."
          );
      });

    const input = terminal.onData((data) => {
      if (socket?.readyState === WebSocket.OPEN)
        socket.send(new TextEncoder().encode(data));
    });
    const resize = () => {
      fitAddon.fit();
      const { cols, rows } = fitAddon.proposeDimensions() ?? {
        cols: 80,
        rows: 24
      };
      if (socket?.readyState === WebSocket.OPEN)
        socket.send(JSON.stringify(buildResizeMessage(cols, rows)));
    };
    const observer = new ResizeObserver(resize);
    observer.observe(containerRef.current!);

    return () => {
      cancelled = true;
      observer.disconnect();
      input.dispose();
      socket?.close();
      terminal.dispose();
    };
  }, [workspaceId]);

  return (
    <div className="relative h-[28rem] overflow-hidden rounded-md bg-black p-2">
      <div className="h-full [&_.xterm]:h-full" ref={containerRef} />
      {error ? (
        <p
          className="absolute inset-0 grid place-items-center bg-black p-6 text-sm text-destructive"
          role="alert"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}
