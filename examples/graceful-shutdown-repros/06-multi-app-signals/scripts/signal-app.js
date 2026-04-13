const fs = require("node:fs")
const path = require("node:path")

const [, , appName, modeEnvVar] = process.argv

if (!appName || !modeEnvVar) {
  throw new Error("usage: node signal-app.js <app-name> <mode-env-var>")
}

const appDir = process.cwd()
const eventsFile = path.join(appDir, "events.log")
const mode = process.env[modeEnvVar] || "graceful"
const supportedModes = new Set(["default", "graceful", "slow", "stubborn"])

if (!supportedModes.has(mode)) {
  throw new Error(
    `unsupported mode ${mode} for ${modeEnvVar}; expected one of ${Array.from(supportedModes).join(", ")}`
  )
}

function write(name, contents) {
  fs.writeFileSync(path.join(appDir, name), contents)
}

function append(message) {
  console.log(message)
  fs.appendFileSync(eventsFile, `${new Date().toISOString()} ${message}\n`)
}

write("pid", `${process.pid}\n`)
write("ready", "ready\n")
append(`${appName} ready mode=${mode} pid=${process.pid}`)

if (mode === "default") {
  append(`${appName} running with no signal handlers`)
  setInterval(() => {}, 1000)
  return
}

let shuttingDown = false

function handleSignal(signal) {
  if (shuttingDown) {
    return
  }

  write(`${signal.toLowerCase()}.txt`, `${signal}\n`)
  append(`${appName} received ${signal}`)

  if (mode === "stubborn") {
    append(`${appName} ignoring ${signal}`)
    return
  }

  shuttingDown = true

  const exitDelayMs = mode === "slow" ? 5000 : 500
  if (mode === "slow") {
    append(`${appName} taking awhile to exit after ${signal}`)
  }

  setTimeout(() => {
    append(`${appName} exiting after ${signal}`)
    process.exit(0)
  }, exitDelayMs)
}

process.on("SIGINT", () => handleSignal("SIGINT"))
process.on("SIGTERM", () => handleSignal("SIGTERM"))

setInterval(() => {}, 1000)
