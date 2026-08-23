"use client";

import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";

import {
  buildResizeMessage,
  buildStartMessage,
  buildWebSocketUrl,
  parseServerMessage,
  shouldReconnectTerminal
} from "../lib/sandbox-terminal-protocol";

const RETRY_DELAY_MS = 2_000;
const MAX_STARTUP_ATTEMPTS = 45;

interface WorkspaceTerminalProps {
  readonly workspaceId: string;
}

export function WorkspaceTerminal({ workspaceId }: WorkspaceTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState("Connecting to sandbox…");
  const [connecting, setConnecting] = useState(true);

  useEffect(() => {
    let cancelled = false;
    let socket: WebSocket | undefined;
    let retryTimer: number | undefined;

    const terminal = new Terminal({
      allowProposedApi: false,
      cursorBlink: true,
      fontFamily: "var(--font-geist-mono), monospace",
      fontSize: 14,
      lineHeight: 1.5,
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
      if (attempt === 0) setStatus("Connecting to sandbox…");

      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}/terminal`,
        {
          method: "POST",
          headers: { "x-operator-action": "open-workspace-terminal" }
        }
      );
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as {
          code?: string;
          error?: string;
        };
        if (
          response.status === 503 &&
          (body.code === "chat_initializing" || body.code === "chat_handoff")
        ) {
          setStatus(
            body.error ?? "Factory is preparing the chat for this terminal."
          );
          retry(attempt);
          return;
        }
        throw new Error(
          body.error ?? `Could not open terminal (${response.status}).`
        );
      }
      const session = (await response.json()) as {
        readonly url: string;
        readonly token: string;
        readonly command: string;
        readonly args: readonly string[];
        readonly cwd: string;
      };
      if (cancelled) return;

      let receivedOutput = false;
      let receivedExit = false;
      const currentSocket = new WebSocket(
        buildWebSocketUrl(session.url, session.token)
      );
      socket = currentSocket;
      currentSocket.binaryType = "arraybuffer";
      currentSocket.addEventListener("open", () => {
        if (cancelled || socket !== currentSocket) return;
        const { cols, rows } = fitAddon.proposeDimensions() ?? {
          cols: 80,
          rows: 24
        };
        terminal.resize(cols, rows);
        currentSocket.send(
          JSON.stringify(
            buildStartMessage(cols, rows, {
              command: session.command,
              args: session.args,
              cwd: session.cwd
            })
          )
        );
        setConnecting(false);
        terminal.focus();
      });
      currentSocket.addEventListener("message", (event) => {
        receivedOutput = true;
        const message = parseServerMessage(event.data);
        if (message.kind === "output") terminal.write(message.data);
        if (message.kind === "exit") {
          receivedExit = true;
          terminal.writeln(
            `\r\nSession closed${message.code === null ? "." : ` with exit code ${message.code}.`}`
          );
        }
      });
      currentSocket.addEventListener("close", (event) => {
        if (cancelled || socket !== currentSocket) return;
        socket = undefined;
        if (!shouldReconnectTerminal(event.code, receivedExit)) return;
        setStatus("Reconnecting to sandbox…");
        setConnecting(!receivedOutput);
        retry(attempt);
      });
      currentSocket.addEventListener("error", () => {
        // The close event owns reconnects so an error cannot schedule twice.
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
          {status}
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
