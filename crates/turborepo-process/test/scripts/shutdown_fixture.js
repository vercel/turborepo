#!/usr/bin/env node
"use strict";

// Deterministic wrapper/worker fixture for the adversarial graceful-shutdown
// tests in crates/turborepo-process/src/child/test/shutdown_adversarial.rs.
//
// Every process installs a SIGINT handler that records each delivery, so a
// wrapper forwarding on top of Turborepo's group signal is an observation
// rather than a silent kill. SIGINT counts and cleanup completion are written
// to files in <stateDir>; the tests treat counts as observations and use the
// file markers as the actual contract.
//
// Usage:
//   worker:  node shutdown_fixture.js worker  <stateDir> <cleanupMs> <watchdogMs>
//   wrapper: node shutdown_fixture.js wrapper <stateDir> <root:0|1> <depth> \
//              <forward:none|delayed> <forwardDelayMs> \
//              <exit:now|after-child|after-ack> <cleanupMs> <watchdogMs>
//
// A wrapper with depth > 0 spawns another wrapper; depth 0 spawns the worker.
// Exit modes:
//   now         record that this wrapper exited and exit immediately.
//   after-child stay alive until the child exits, then mirror its status.
//   after-ack   nonforwarding only: wait until the worker has acknowledged the
//               group SIGINT, then exit. This is a file readiness barrier, not
//               a sleep, so a subsequent remaining-descendant re-signal cannot
//               be coalesced with the original delivery.
//
// Readiness: every process writes "ready"/"wrapper-ready-<depth>" after its
// handlers are installed, and the outermost wrapper (root=1) writes
// "tree-ready" once the whole tree is ready. Tests await that marker so they
// never signal before every handler exists.

const fs = require("fs");
const path = require("path");
const { spawn } = require("child_process");

// During shutdown Turborepo may stop draining our stdout, and a PTY session
// leader exiting hangs up the terminal for its process group. Those teardown
// errors (EPIPE/EIO/ERR_STREAM_DESTROYED) are expected and swallowed. Any other
// stream error is recorded and aborts the fixture so it cannot be hidden.
for (const stream of [process.stdout, process.stderr]) {
  stream.on("error", (err) => {
    const code = err && err.code;
    if (
      code === "EPIPE" ||
      code === "EIO" ||
      code === "ERR_STREAM_DESTROYED" ||
      code === "ERR_STREAM_WRITE_AFTER_END"
    ) {
      return;
    }
    write(`io-error-${process.pid}`, `${code || err}\n`);
    process.exit(70);
  });
}

// PTY children run as session leaders. When one exits the kernel can deliver
// SIGHUP to the remaining process group, which would otherwise cut a worker's
// file-based cleanup proof short. The handler keeps a hangup from ending a
// fixture process and records it per process. SIGHUP is deliberately not
// treated as a shutdown signal.
process.on("SIGHUP", () => {
  write(`sighup-${process.pid}`, "1\n");
});

const [, , role, stateDir, ...rest] = process.argv;

function write(file, contents) {
  fs.writeFileSync(path.join(stateDir, file), contents);
}

function log(line) {
  console.log(line);
}

function keepAlive() {
  setInterval(() => {}, 1000);
}

function worker(cleanupMs, watchdogMs) {
  let sigints = 0;
  let cleaning = false;

  process.on("SIGINT", () => {
    sigints += 1;
    write("worker-sigint-count", `${sigints}\n`);
    log(`worker sigint count=${sigints}`);
    if (cleaning) return;
    cleaning = true;
    setTimeout(() => {
      // Marker first, log second: the file is the proof even if the terminal
      // has already been hung up.
      write("worker-cleanup-complete", `${sigints}\n`);
      log("worker cleanup complete");
      process.exit(0);
    }, cleanupMs);
  });

  setTimeout(() => {
    write("worker-watchdog", "1\n");
    process.exit(3);
  }, watchdogMs);

  write("worker-sigint-count", "0\n");
  write("ready", `${process.pid}\n`);
  log(`worker started pid=${process.pid}`);

  keepAlive();
}

function wrapper(root, depth, forward, forwardDelayMs, exitMode, cleanupMs, watchdogMs) {
  const childArgs =
    depth === 0
      ? [__filename, "worker", stateDir, String(cleanupMs), String(watchdogMs)]
      : [
          __filename,
          "wrapper",
          stateDir,
          "0",
          String(depth - 1),
          forward,
          String(forwardDelayMs),
          exitMode,
          String(cleanupMs),
          String(watchdogMs),
        ];

  const child = spawn(process.execPath, childArgs, { stdio: "inherit" });
  let sigints = 0;
  let acted = false;

  const markExit = (reason) => {
    write(`wrapper-exited-${depth}`, "1\n");
    if (root === "1") write("root-exited", "1\n");
    log(`wrapper exit pid=${process.pid} depth=${depth} reason=${reason}`);
    process.exit(0);
  };

  // File readiness barrier: only leave once the worker has recorded the first
  // interrupt, so the remaining-descendant re-signal is a distinct delivery.
  const waitForWorkerAck = (onAck) => {
    const ackPath = path.join(stateDir, "worker-sigint-count");
    const deadline = Date.now() + 5000;
    const timer = setInterval(() => {
      let acked = false;
      try {
        acked = Number.parseInt(fs.readFileSync(ackPath, "utf8").trim(), 10) >= 1;
      } catch (_err) {
        acked = false;
      }
      if (acked) {
        clearInterval(timer);
        onAck();
      } else if (Date.now() > deadline) {
        clearInterval(timer);
        write(`wrapper-ack-timeout-${depth}`, "1\n");
        log(`wrapper ack timeout pid=${process.pid} depth=${depth}`);
        process.exit(2);
      }
    }, 5);
  };

  process.on("SIGINT", () => {
    sigints += 1;
    write(`wrapper-${process.pid}-sigint-count`, `${sigints}\n`);
    log(`wrapper sigint pid=${process.pid} depth=${depth} count=${sigints}`);
    if (acted) return;
    acted = true;

    if (forward === "delayed") {
      setTimeout(() => {
        write(`wrapper-${process.pid}-forwarded`, "1\n");
        child.kill("SIGINT");
        if (exitMode === "now") markExit("forwarded-early");
      }, forwardDelayMs);
      return;
    }

    // forward === "none": this wrapper never signals its child.
    if (exitMode === "now") {
      markExit("nonforwarding-early");
    } else if (exitMode === "after-ack") {
      waitForWorkerAck(() => markExit("nonforwarding-after-ack"));
    }
  });

  child.on("exit", (code, signal) => {
    log(`wrapper child exit pid=${process.pid} code=${code} signal=${signal}`);
    process.exit(code ?? (signal ? 1 : 0));
  });

  write(`wrapper-ready-${depth}`, `${process.pid}\n`);
  log(`wrapper ready pid=${process.pid} depth=${depth}`);

  if (root === "1") {
    const readyPath = path.join(stateDir, "ready");
    const wrappersReady = () =>
      Array.from({ length: depth + 1 }, (_, d) =>
        fs.existsSync(path.join(stateDir, `wrapper-ready-${d}`)),
      ).every(Boolean);
    const readyDeadline = Date.now() + 10000;
    const readyTimer = setInterval(() => {
      if (fs.existsSync(readyPath) && wrappersReady()) {
        clearInterval(readyTimer);
        write("tree-ready", `${process.pid}\n`);
        log("tree ready");
      } else if (Date.now() > readyDeadline) {
        clearInterval(readyTimer);
        log(`wrapper ready timeout pid=${process.pid}`);
        process.exit(2);
      }
    }, 10);
  }

  setTimeout(() => {
    write(`wrapper-${process.pid}-watchdog`, "1\n");
    process.exit(3);
  }, watchdogMs);

  keepAlive();
}

if (role === "worker") {
  worker(Number(rest[0]), Number(rest[1]));
} else if (role === "wrapper") {
  const [root, depth, forward, forwardDelayMs, exitMode, cleanupMs, watchdogMs] = rest;
  wrapper(
    root,
    Number(depth),
    forward,
    Number(forwardDelayMs),
    exitMode,
    Number(cleanupMs),
    Number(watchdogMs),
  );
} else {
  console.error(`unknown role: ${role}`);
  process.exit(64);
}
