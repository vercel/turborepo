process.stdin.setEncoding("utf8")
process.stdin.resume()

console.log("INTERACTIVE_READY")

process.stdin.on("data", (input) => {
  const value = input.replace(/[\r\n]+/g, "")
  if (value) console.log(`INTERACTIVE_ECHO:${value}`)
})

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => process.exit(0))
}
