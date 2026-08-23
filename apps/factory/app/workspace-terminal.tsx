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

interface WorkspaceTerminalProps {
  readonly workspaceId: string;
}

export function WorkspaceTerminal({ workspaceId }: WorkspaceTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let socket: WebSocket | undefined;
    let terminal: Terminal | undefined;
    let fitAddon: FitAddon | undefined;
    let resizeObserver: ResizeObserver | undefined;
    let handshakeTimer: number | undefined;

    async function connect() {
      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}/terminal`,
        {
          method: "POST",
          headers: { "x-operator-action": "open-workspace-terminal" }
        }
      );
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as {
          error?: string;
        };
        throw new Error(
          body.error ?? `Could not open terminal (${response.status}).`
        );
      }
      const session = (await response.json()) as {
        readonly url: string;
        readonly token: string;
        readonly cwd: string;
      };
      if (cancelled) return;

      terminal = new Terminal({
        allowProposedApi: false,
        cursorBlink: true,
        fontFamily: "var(--font-geist-mono), monospace",
        fontSize: 14,
        theme: { background: "#000000", foreground: "#ededed" }
      });
      fitAddon = new FitAddon();
      terminal.loadAddon(fitAddon);
      if (containerRef.current) {
        terminal.open(containerRef.current);
        fitAddon.fit();
      }

      socket = new WebSocket(buildWebSocketUrl(session.url, session.token));
      socket.binaryType = "arraybuffer";
      socket.addEventListener("open", () => {
        if (cancelled || !terminal || !fitAddon || !socket) return;
        const { cols, rows } = fitAddon.proposeDimensions() ?? {
          cols: 80,
          rows: 24
        };
        terminal.resize(cols, rows);
        socket.send(
          JSON.stringify(buildStartMessage(cols, rows, { cwd: session.cwd }))
        );
        handshakeTimer = window.setTimeout(() => {
          setError("The sandbox shell did not start.");
          socket?.close();
        }, 10_000);
        terminal.focus();
      });
      socket.addEventListener("message", (event) => {
        if (!terminal) return;
        if (handshakeTimer !== undefined) {
          window.clearTimeout(handshakeTimer);
          handshakeTimer = undefined;
        }
        const message = parseServerMessage(event.data);
        if (message.kind === "output") terminal.write(message.data);
        if (message.kind === "exit") {
          terminal.writeln(
            `\r\nSession closed${message.code === null ? "." : ` with exit code ${message.code}.`}`
          );
        }
      });
      socket.addEventListener("close", (event) => {
        if (!cancelled && event.code !== 1000)
          setError(`The terminal connection closed (${event.code}).`);
      });
      socket.addEventListener("error", () => {
        if (!cancelled) setError("The terminal connection failed.");
      });
      terminal.onData((data) => {
        if (socket?.readyState === WebSocket.OPEN)
          socket.send(new TextEncoder().encode(data));
      });

      const resize = () => {
        if (!terminal || !fitAddon) return;
        fitAddon.fit();
        const { cols, rows } = fitAddon.proposeDimensions() ?? {
          cols: 80,
          rows: 24
        };
        terminal.resize(cols, rows);
        if (socket?.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify(buildResizeMessage(cols, rows)));
        }
      };
      if (containerRef.current) {
        resizeObserver = new ResizeObserver(resize);
        resizeObserver.observe(containerRef.current);
      }
    }

    void connect().catch((cause) => {
      if (!cancelled) {
        setError(
          cause instanceof Error ? cause.message : "Could not start terminal."
        );
      }
    });

    return () => {
      cancelled = true;
      if (handshakeTimer !== undefined) window.clearTimeout(handshakeTimer);
      resizeObserver?.disconnect();
      socket?.close();
      terminal?.dispose();
    };
  }, [workspaceId]);

  return (
    <main
      aria-label="Sandbox terminal"
      className="fixed inset-0 z-50 bg-black text-[#ededed]"
      id="main-content"
    >
      {error ? (
        <p className="grid h-full place-items-center p-6 text-sm" role="alert">
          {error}
        </p>
      ) : (
        <div
          className="h-full p-2 [&_.xterm]:h-full [&_.xterm-screen]:!h-full [&_.xterm-screen]:!w-full"
          ref={containerRef}
        />
      )}
    </main>
  );
}
