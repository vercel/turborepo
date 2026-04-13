const fs = require("node:fs")
const path = require("node:path")

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

log(`node worker ready pid=${process.pid}`)

let shuttingDown = false

function handleSignal(signal) {
  if (shuttingDown) {
    return
  }

  write(`${signal.toLowerCase()}.worker`, `${signal}\n`)
  log(`node worker received ${signal}`)

  if (process.env.WORKER_STUBBORN === "1") {
    log(`node worker staying alive after ${signal}`)
    return
  }

  shuttingDown = true
  process.exit(0)
}

process.on("SIGINT", () => handleSignal("SIGINT"))
process.on("SIGTERM", () => handleSignal("SIGTERM"))

setInterval(() => {}, 1000)
