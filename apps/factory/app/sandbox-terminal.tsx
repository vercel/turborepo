"use client";

import { useEffect, useEffectEvent, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import {
  buildWebSocketUrl,
  buildStartMessage,
  buildResizeMessage,
  parseServerMessage,
  DEFAULT_CWD
} from "@/lib/sandbox-terminal-protocol";

export interface SandboxTerminalProps {
  readonly sandboxName: string;
  readonly onExit: () => void;
}

export function SandboxTerminal({ sandboxName, onExit }: SandboxTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const exitedRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(true);
  const onExitEvent = useEffectEvent(onExit);

  useEffect(() => {
    let cancelled = false;

    async function setup() {
      const response = await fetch("/api/sandbox/terminal", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ sandboxName })
      });

      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as {
          error?: string;
        };
        throw new Error(
          body.error ?? `Could not open terminal session (${response.status}).`
        );
      }

      const session = (await response.json()) as {
        readonly url: string;
        readonly token: string;
      };

      if (cancelled) return;

      const terminal = new Terminal({
        cursorBlink: true,
        fontFamily: "var(--font-geist-mono), monospace",
        fontSize: 14,
        theme: {
          background: "#000000",
          foreground: "#ededed"
        }
      });

      const fitAddon = new FitAddon();
      terminal.loadAddon(fitAddon);

      if (containerRef.current) {
        terminal.open(containerRef.current);
        fitAddon.fit();
      }

      terminalRef.current = terminal;
      fitAddonRef.current = fitAddon;

      const socket = new WebSocket(
        buildWebSocketUrl(session.url, session.token)
      );
      socketRef.current = socket;
      socket.binaryType = "arraybuffer";

      socket.addEventListener("open", () => {
        if (cancelled) return;
        setConnecting(false);
        const { cols, rows } = fitAddon.proposeDimensions() ?? {
          cols: 80,
          rows: 24
        };
        terminal.resize(cols, rows);
        socket.send(
          JSON.stringify(buildStartMessage(cols, rows, { cwd: DEFAULT_CWD }))
        );
      });

      socket.addEventListener("message", (event) => {
        const message = parseServerMessage(event.data);
        if (message.kind === "exit") {
          exitedRef.current = true;
          terminal.writeln("");
          terminal.writeln(
            `Connection to ${sandboxName} closed.` +
              (message.code !== null ? ` Exit code: ${message.code}.` : "")
          );
          setTimeout(() => {
            if (!cancelled) onExitEvent();
          }, 600);
          return;
        }
        if (message.kind === "output") {
          terminal.write(message.data);
        }
      });

      terminal.onData((data) => {
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(new TextEncoder().encode(data));
        }
      });

      const onResize = () => {
        if (!fitAddonRef.current || !terminalRef.current) return;
        fitAddonRef.current.fit();
        const { cols, rows } = fitAddonRef.current.proposeDimensions() ?? {
          cols: 80,
          rows: 24
        };
        terminalRef.current.resize(cols, rows);
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify(buildResizeMessage(cols, rows)));
        }
      };

      window.addEventListener("resize", onResize);

      socket.addEventListener("close", () => {
        if (!exitedRef.current && !cancelled) {
          setError("The terminal session was closed unexpectedly.");
        }
      });

      socket.addEventListener("error", () => {
        if (!cancelled) {
          setError("The terminal connection failed.");
        }
      });

      return () => {
        window.removeEventListener("resize", onResize);
      };
    }

    const cleanupPromise = setup().catch((err) => {
      if (!cancelled) {
        setError(
          err instanceof Error ? err.message : "Could not start terminal."
        );
      }
    });

    return () => {
      cancelled = true;
      socketRef.current?.close();
      terminalRef.current?.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      socketRef.current = null;
      void cleanupPromise;
    };
  }, [sandboxName]);

  if (error) {
    return (
      <div className="sandboxTerminalOverlay sandboxTerminalOverlay-error">
        <div className="sandboxTerminalNotice" role="alert">
          <p>{error}</p>
          <button
            className="sandboxTerminalClose"
            onClick={onExit}
            type="button"
          >
            Return to Factory
          </button>
        </div>
      </div>
    );
  }

  return (
    <div
      className="sandboxTerminalOverlay"
      role="dialog"
      aria-label={`Terminal for ${sandboxName}`}
    >
      {connecting ? (
        <div className="sandboxTerminalNotice" role="status">
          <p>Opening terminal session for {sandboxName}…</p>
        </div>
      ) : null}
      <div ref={containerRef} className="sandboxTerminalContainer" />
    </div>
  );
}
