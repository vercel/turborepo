function delay(time) {
  return new Promise((resolve) => setTimeout(resolve, time))
}

let shuttingDown = false

process.on("SIGINT", async () => {
  if (shuttingDown) {
    return
  }

  shuttingDown = true
  console.log("received SIGINT")
  await delay(10)
  console.log("exiting after SIGINT")
  process.exit(0)
})

console.log("ready")
setInterval(() => {}, 1000)
