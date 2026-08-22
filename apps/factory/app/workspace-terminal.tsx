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
import { Button } from "../components/ui/button";

interface WorkspaceTerminalProps {
  readonly workspaceId: string;
  readonly workspaceTitle: string;
  readonly onExit: () => void;
}

export function WorkspaceTerminal({
  workspaceId,
  workspaceTitle,
  onExit
}: WorkspaceTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const onExitRef = useRef(onExit);
  const [error, setError] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(true);
  onExitRef.current = onExit;

  useEffect(() => {
    let cancelled = false;
    let socket: WebSocket | undefined;
    let terminal: Terminal | undefined;
    let fitAddon: FitAddon | undefined;

    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onExitRef.current();
    };
    window.addEventListener("keydown", closeOnEscape);

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
        setConnecting(false);
        const { cols, rows } = fitAddon.proposeDimensions() ?? {
          cols: 80,
          rows: 24
        };
        terminal.resize(cols, rows);
        socket.send(
          JSON.stringify(buildStartMessage(cols, rows, { cwd: session.cwd }))
        );
        terminal.focus();
      });
      socket.addEventListener("message", (event) => {
        if (!terminal) return;
        const message = parseServerMessage(event.data);
        if (message.kind === "output") terminal.write(message.data);
        if (message.kind === "exit") {
          terminal.writeln(
            `\r\nSession closed${message.code === null ? "." : ` with exit code ${message.code}.`}`
          );
        }
      });
      socket.addEventListener("close", () => {
        if (!cancelled) setConnecting(false);
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
      window.addEventListener("resize", resize);
      return () => window.removeEventListener("resize", resize);
    }

    let removeResize: (() => void) | undefined;
    void connect()
      .then((cleanup) => {
        if (cancelled) cleanup?.();
        else removeResize = cleanup;
      })
      .catch((cause) => {
        if (!cancelled) {
          setConnecting(false);
          setError(
            cause instanceof Error ? cause.message : "Could not start terminal."
          );
        }
      });

    return () => {
      cancelled = true;
      removeResize?.();
      window.removeEventListener("keydown", closeOnEscape);
      socket?.close();
      terminal?.dispose();
    };
  }, [workspaceId]);

  return (
    <div
      aria-label={`Terminal for ${workspaceTitle}`}
      aria-modal="true"
      className="fixed inset-0 z-50 flex flex-col bg-black text-[#ededed]"
      role="dialog"
    >
      <header className="flex min-h-12 items-center justify-between gap-4 border-b border-white/20 px-4">
        <span className="truncate font-mono text-xs">{workspaceTitle}</span>
        <Button
          className="text-white hover:bg-white/15 hover:text-white"
          onClick={onExit}
          size="sm"
          type="button"
          variant="ghost"
        >
          Close <span className="sr-only">terminal</span>
        </Button>
      </header>
      {connecting ? (
        <p
          className="pointer-events-none absolute inset-x-0 top-12 bottom-0 grid place-items-center text-sm"
          role="status"
        >
          Opening terminal…
        </p>
      ) : null}
      {error ? (
        <div
          className="grid flex-1 place-items-center p-6 text-center"
          role="alert"
        >
          <div>
            <p>{error}</p>
            <Button className="mt-4" onClick={onExit} type="button">
              Return to workspace
            </Button>
          </div>
        </div>
      ) : (
        <div
          className="min-h-0 flex-1 p-2 [&_.xterm]:h-full [&_.xterm-screen]:!h-full [&_.xterm-screen]:!w-full"
          ref={containerRef}
        />
      )}
    </div>
  );
}
