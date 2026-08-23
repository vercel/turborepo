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

const RETRY_DELAY_MS = 2_000;
const MAX_STARTUP_ATTEMPTS = 45;

interface WorkspaceTerminalProps {
  readonly workspaceId: string;
}

export function WorkspaceTerminal({ workspaceId }: WorkspaceTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(true);

  useEffect(() => {
    let cancelled = false;
    let socket: WebSocket | undefined;
    let handshakeTimer: number | undefined;
    let retryTimer: number | undefined;

    const terminal = new Terminal({
      allowProposedApi: false,
      cursorBlink: true,
      fontFamily: "var(--font-geist-mono), monospace",
      fontSize: 14,
      theme: { background: "#000000", foreground: "#ededed" }
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    if (containerRef.current) {
      terminal.open(containerRef.current);
      fitAddon.fit();
    }

    const retry = (attempt: number) => {
      if (cancelled || attempt >= MAX_STARTUP_ATTEMPTS) {
        if (!cancelled) setError("The sandbox terminal did not become ready.");
        return;
      }
      retryTimer = window.setTimeout(() => {
        void connect(attempt + 1).catch((cause) => {
          if (!cancelled) {
            setConnecting(false);
            setError(
              cause instanceof Error
                ? cause.message
                : "Could not start terminal."
            );
          }
        });
      }, RETRY_DELAY_MS);
    };

    async function connect(attempt: number) {
      if (cancelled) return;
      setConnecting(true);
      setError(null);

      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}/terminal`,
        {
          method: "POST",
          headers: { "x-operator-action": "open-workspace-terminal" }
        }
      );
      if (!response.ok) {
        if (response.status === 503) {
          retry(attempt);
          return;
        }
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

      let receivedOutput = false;
      socket = new WebSocket(buildWebSocketUrl(session.url, session.token));
      socket.binaryType = "arraybuffer";
      socket.addEventListener("open", () => {
        if (cancelled || !socket) return;
        const { cols, rows } = fitAddon.proposeDimensions() ?? {
          cols: 80,
          rows: 24
        };
        terminal.resize(cols, rows);
        socket.send(
          JSON.stringify(buildStartMessage(cols, rows, { cwd: session.cwd }))
        );
        handshakeTimer = window.setTimeout(() => {
          socket?.close();
          retry(attempt);
        }, 10_000);
        terminal.focus();
      });
      socket.addEventListener("message", (event) => {
        receivedOutput = true;
        setConnecting(false);
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
        if (cancelled) return;
        if (!receivedOutput && event.code === 1006) {
          retry(attempt);
          return;
        }
        if (event.code !== 1000)
          setError(`The terminal connection closed (${event.code}).`);
      });
      socket.addEventListener("error", () => {
        if (!cancelled && receivedOutput)
          setError("The terminal connection failed.");
      });
    }

    const dataDisposable = terminal.onData((data) => {
      if (socket?.readyState === WebSocket.OPEN)
        socket.send(new TextEncoder().encode(data));
    });

    const resize = () => {
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
    const resizeObserver = new ResizeObserver(resize);
    if (containerRef.current) resizeObserver.observe(containerRef.current);

    void connect(0).catch((cause) => {
      if (!cancelled) {
        setConnecting(false);
        setError(
          cause instanceof Error ? cause.message : "Could not start terminal."
        );
      }
    });

    return () => {
      cancelled = true;
      if (handshakeTimer !== undefined) window.clearTimeout(handshakeTimer);
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      resizeObserver.disconnect();
      dataDisposable.dispose();
      socket?.close();
      terminal.dispose();
    };
  }, [workspaceId]);

  return (
    <main
      aria-label="Sandbox terminal"
      className="fixed inset-0 z-50 bg-black text-[#ededed]"
      id="main-content"
    >
      <div
        className="h-full p-2 [&_.xterm]:h-full [&_.xterm-screen]:!h-full [&_.xterm-screen]:!w-full"
        ref={containerRef}
      />
      {connecting && !error ? (
        <p
          className="pointer-events-none absolute inset-0 grid place-items-center text-sm"
          role="status"
        >
          Connecting to sandbox…
        </p>
      ) : null}
      {error ? (
        <p
          className="absolute inset-0 grid place-items-center bg-black p-6 text-sm"
          role="alert"
        >
          {error}
        </p>
      ) : null}
    </main>
  );
}
