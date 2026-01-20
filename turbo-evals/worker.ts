#!/usr/bin/env bun

import { parentPort, workerData } from "worker_threads";
import { runEval } from "./lib/eval-runner";

interface WorkerData {
  evalPath: string;
  dryRun: boolean;
  verbose: boolean;
  debug: boolean;
}

async function runWorker() {
  if (!parentPort || !workerData) {
    throw new Error("Worker must be run with parentPort and workerData");
  }

  const { evalPath, dryRun, verbose, debug }: WorkerData = workerData;

  try {
    const result = await runEval(evalPath, dryRun, verbose, debug);

    parentPort.postMessage({
      success: true,
      evalPath,
      result
    });
  } catch (error) {
    parentPort.postMessage({
      success: false,
      evalPath,
      error: error instanceof Error ? error.message : String(error)
    });
  }
}

// Run the worker
runWorker().catch((error) => {
  if (parentPort) {
    parentPort.postMessage({
      success: false,
      evalPath: workerData?.evalPath || "unknown",
      error: error instanceof Error ? error.message : String(error)
    });
  }
});
