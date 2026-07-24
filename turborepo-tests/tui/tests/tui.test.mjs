import assert from "node:assert/strict"
import { cp, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

import { TerminalControl } from "@kitlangton/terminal-control"

const packageDirectory = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const repositoryRoot = resolve(packageDirectory, "../..")
const fixtureSource = join(packageDirectory, "fixtures/basic")
const turboBinary = process.env.TURBO_BINARY_PATH ?? join(repositoryRoot, "target/debug/turbo")
const startupTimeout = 15_000
// Known failing regressions stay skipped in CI but remain directly reproducible.
const runKnownBugs = process.env.TURBO_TEST_KNOWN_BUGS === "1"

function inheritedEnvironment() {
  const environment = {
    PATH: process.env.PATH,
    HOME: process.env.HOME,
    LANG: process.env.LANG,
    SHELL: process.env.SHELL,
    TMPDIR: process.env.TMPDIR,
    USER: process.env.USER,
    COREPACK_ENABLE_DOWNLOAD_PROMPT: "0",
    TURBO_GLOBAL_WARNING_DISABLED: "1",
    TURBO_PRINT_VERSION_DISABLED: "1",
    TURBO_TELEMETRY_MESSAGE_DISABLED: "1",
    DO_NOT_TRACK: "1",
    NPM_CONFIG_UPDATE_NOTIFIER: "false",
  }

  return Object.fromEntries(Object.entries(environment).filter(([, value]) => value !== undefined))
}

async function launchTui({
  extraArgs = [],
  prepareWorkspace,
  task = "dev",
  viewport = { cols: 100, rows: 30 },
} = {}) {
  const directory = await mkdtemp(join(tmpdir(), "turbo-tui-test-"))
  const workspace = join(directory, "workspace")
  await cp(fixtureSource, workspace, { recursive: true })
  await prepareWorkspace?.(workspace)

  const configDirectory = join(directory, "config")
  await mkdir(configDirectory)

  let terminal
  try {
    terminal = await TerminalControl.make()
    const session = await terminal.launch({
      command: [turboBinary, "run", task, "--ui=tui", "--skip-infer", ...extraArgs],
      cwd: workspace,
      viewport,
      inheritEnv: false,
      env: {
        ...inheritedEnvironment(),
        TURBO_CONFIG_DIR_PATH: configDirectory,
      },
    })

    return { directory, session, terminal }
  } catch (error) {
    await terminal?.close().catch(() => {})
    await rm(directory, { force: true, recursive: true })
    throw error
  }
}

async function closeTui(context) {
  try {
    await context.session.stop()
  } finally {
    try {
      await context.terminal.close()
    } finally {
      await rm(context.directory, { force: true, recursive: true })
    }
  }
}

async function waitForScreen(session, description, predicate, timeoutMs = startupTimeout) {
  try {
    return await session.screen.waitUntil(
      (screen) => predicate(screen.text, screen.frame),
      { timeoutMs },
    )
  } catch (error) {
    const screen = await session.screen.capture({ allowIncomplete: true, deadlineMs: 0, settleMs: 0 })
    throw new Error(`${description}\n\nVisible screen:\n${screen.text}`, { cause: error })
  }
}

async function addOverflowingTaskList(workspace) {
  await writeFile(
    join(workspace, "turbo.json"),
    JSON.stringify({
      $schema: "https://turborepo.dev/schema.json",
      ui: "tui",
      tasks: { list: { cache: false } },
    }),
  )

  for (let index = 0; index < 20; index++) {
    const name = `zz-task-${String(index).padStart(2, "0")}`
    const packageDirectory = join(workspace, "packages", name)
    await mkdir(packageDirectory)
    await writeFile(
      join(packageDirectory, "package.json"),
      JSON.stringify({
        name,
        private: true,
        scripts: { list: "node -e \"setTimeout(() => {}, 30000)\"" },
      }),
    )
  }
}

async function waitForTranscriptIdle(session, quietForMs = 500, timeoutMs = startupTimeout) {
  const deadline = Date.now() + timeoutMs
  let transcript = await session.transcript.ansi()
  let quietSince = Date.now()

  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50))
    const next = await session.transcript.ansi()
    if (next.byteLength !== transcript.byteLength) {
      transcript = next
      quietSince = Date.now()
    } else if (Date.now() - quietSince >= quietForMs) {
      return next
    }
  }

  throw new Error("terminal output did not become idle")
}

function visibleSidebarTasks(text) {
  return text.split("\n").flatMap((line) => {
    const divider = line.indexOf("│")
    if (divider < 0) return []
    return line.slice(0, divider).match(/[a-z0-9-]+#list/g) ?? []
  })
}

async function waitForInitialScreen(session) {
  return waitForScreen(session, "TUI did not render its initial task view", (text) =>
    text.includes("Tasks (/ - Search)") &&
    text.includes("interactive#dev") &&
    text.includes("finite#dev") &&
    text.includes("INTERACTIVE_READY")
  )
}

test("renders tasks and navigates between their output", { timeout: 30_000 }, async () => {
  const context = await launchTui()
  try {
    await waitForInitialScreen(context.session)

    await context.session.keyboard.press("ArrowDown")
    await waitForScreen(context.session, "finite task output was not selected", (text) =>
      text.includes("FINITE_COMPLETE")
    )

    await context.session.keyboard.press("ArrowUp")
    await waitForScreen(context.session, "interactive task output was not reselected", (text) =>
      text.includes("INTERACTIVE_READY")
    )
  } finally {
    await closeTui(context)
  }
})

test("drives search and popup overlays", { timeout: 30_000 }, async () => {
  const context = await launchTui()
  try {
    await waitForInitialScreen(context.session)

    await context.session.keyboard.type("/finite")
    await context.session.keyboard.press("Enter")
    await waitForScreen(context.session, "search did not select the finite task", (text) =>
      text.includes("/ finite") && text.includes("FINITE_COMPLETE")
    )

    await context.session.keyboard.press("Escape")
    await context.session.keyboard.type("m")
    await waitForScreen(context.session, "keybind popup did not open", (text) =>
      text.includes("Keybinds") && text.includes("Toggle log panel")
    )

    await context.session.keyboard.press("Escape")
    await context.session.keyboard.type("l")
    await waitForScreen(context.session, "log panel did not open", (text) => text.includes("l to close"))
    await context.session.keyboard.press("Escape")
  } finally {
    await closeTui(context)
  }
})

test("forwards input to an interactive task and returns to navigation", { timeout: 30_000 }, async () => {
  const context = await launchTui()
  try {
    await waitForInitialScreen(context.session)

    await context.session.keyboard.type("i")
    await waitForScreen(context.session, "interactive mode did not start", (text) =>
      text.includes("Ctrl-z - Stop interacting")
    )

    await context.session.keyboard.type("hello-from-test")
    await context.session.keyboard.press("Enter")
    await waitForScreen(context.session, "task did not receive interactive input", (text) =>
      text.includes("INTERACTIVE_ECHO:hello-from-test")
    )

    await context.session.keyboard.press("Control+Z")
    await waitForScreen(context.session, "interactive mode did not return to navigation", (text) =>
      text.includes("Tasks (/ - Search)") && !text.includes("Ctrl-z - Stop interacting")
    )
  } finally {
    await closeTui(context)
  }
})

test("streams logs and restores the terminal on shutdown", { timeout: 30_000 }, async () => {
  const context = await launchTui()
  try {
    await waitForInitialScreen(context.session)

    await context.session.keyboard.type("h")
    await waitForScreen(context.session, "selected task logs did not start streaming", (text) =>
      text.includes("Streaming logs for interactive#dev. Press h to return to the TUI.") &&
      text.includes("INTERACTIVE_READY")
    )

    await context.session.keyboard.type("h")
    await waitForScreen(context.session, "TUI did not return after streaming", (text) =>
      text.includes("INTERACTIVE_READY") && !text.includes("Streaming logs for interactive#dev")
    )

    await context.session.keyboard.press("Control+C")
    const exit = await context.session.waitForExit({ timeoutMs: 10_000 })
    assert.equal(exit.reason, "exited")

    const restored = await context.session.screen.capture({
      allowIncomplete: true,
      deadlineMs: 0,
      settleMs: 0,
    })
    assert.doesNotMatch(restored.text, /Tasks \(\/ - Search\)/)
  } finally {
    await closeTui(context)
  }
})

test(
  "scrolls the task list with the mouse wheel over the sidebar",
  { skip: runKnownBugs ? false : "known bug", timeout: 30_000 },
  async () => {
    const context = await launchTui({
      extraArgs: ["--concurrency=10"],
      prepareWorkspace: addOverflowingTaskList,
      task: "list",
      viewport: { cols: 100, rows: 15 },
    })
    try {
      await waitForScreen(context.session, "overflowing task list did not render", (text) =>
        text.includes("Tasks (/ - Search)") && visibleSidebarTasks(text).length >= 8
      )

      await waitForTranscriptIdle(context.session)
      const beforeScroll = await context.session.screen.capture({
        allowIncomplete: true,
        deadlineMs: 0,
        settleMs: 0,
      })
      const beforeTasks = visibleSidebarTasks(beforeScroll.text)

      const scrollDownOverSidebar = new TextEncoder().encode("\x1b[<65;5;5M".repeat(5))
      await context.session.keyboard.write(scrollDownOverSidebar)
      await new Promise((resolve) => setTimeout(resolve, 500))

      const afterScroll = await context.session.screen.capture({
        allowIncomplete: true,
        deadlineMs: 0,
        settleMs: 0,
      })
      assert.notDeepEqual(
        visibleSidebarTasks(afterScroll.text),
        beforeTasks,
        "mouse-wheel input over the sidebar did not scroll the task list",
      )
    } finally {
      await closeTui(context)
    }
  },
)

test(
  "repaints the TUI immediately after a terminal resize",
  { timeout: 30_000 },
  async () => {
    const context = await launchTui()
    try {
      await waitForInitialScreen(context.session)
      const beforeResize = await waitForTranscriptIdle(context.session)

      for (let step = 0; step <= 10; step++) {
        await context.session.resize({ cols: 110 + step, rows: 26 + step })
        await new Promise((resolve) => setTimeout(resolve, 10))
      }
      await new Promise((resolve) => setTimeout(resolve, 750))

      const afterResize = await context.session.transcript.ansi()
      const repaint = afterResize.slice(beforeResize.byteLength)
      assert.ok(repaint.byteLength > 0, "Turbo emitted no terminal output in response to the resize")
      assert.ok(new TextDecoder().decode(repaint).includes("\x1b[2J"), "Turbo did not clear and repaint the resized terminal")

      const screen = await context.session.screen.capture({
        allowIncomplete: true,
        deadlineMs: 0,
        settleMs: 0,
      })
      assert.equal(screen.frame.cols, 120)
      assert.equal(screen.frame.rows, 36)
    } finally {
      await closeTui(context)
    }
  },
)
