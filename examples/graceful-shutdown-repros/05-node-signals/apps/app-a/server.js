const fs = require("node:fs")
const path = require("node:path")
const { spawn } = require("node:child_process")

const root = __dirname
const logFile = "events.log"

function write(name, contents) {
  fs.writeFileSync(path.join(root, name), contents)
}

function append(name, contents) {
  fs.appendFileSync(path.join(root, name), contents)
}

function log(message) {
  console.log(message)
  append(logFile, `${new Date().toISOString()} ${message}\n`)
}

try {
  process.on("SIGKILL", () => {
    log("this will never run")
  })
} catch (error) {
  write("sigkill-unhandleable.txt", `${error.message}\n`)
}

const worker = spawn(process.execPath, [path.join(root, "worker.js")], {
  stdio: "inherit",
  env: process.env,
})

write("parent.pid", `${process.pid}\n`)
write("worker.pid", `${worker.pid}\n`)
write("ready", "ready\n")
log(`node parent ready pid=${process.pid} worker=${worker.pid}`)

let shuttingDown = false

function handleSignal(signal) {
  if (shuttingDown) {
    return
  }

  write(`${signal.toLowerCase()}.parent`, `${signal}\n`)
  log(`node parent received ${signal}`)

  if (process.env.NODE_STUBBORN === "1") {
    log(`node parent staying alive after ${signal}`)
    return
  }

  shuttingDown = true
  setTimeout(() => {
    log(`node parent exiting after ${signal}`)
    process.exit(0)
  }, 500)
}

process.on("SIGINT", () => handleSignal("SIGINT"))
process.on("SIGTERM", () => handleSignal("SIGTERM"))

worker.on("exit", (code, signal) => {
  log(`node worker exit code=${code} signal=${signal}`)
})

setInterval(() => {}, 1000)
