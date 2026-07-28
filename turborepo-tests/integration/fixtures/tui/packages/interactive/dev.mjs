process.stdin.setEncoding("utf8")
process.stdin.resume()

console.log("INTERACTIVE_READY")
console.log("\u001b[38;2;12;200;123mCOLOR_OUTPUT\u001b[0m")

process.stdin.on("data", (input) => {
  const value = input.replace(/[\r\n]+/g, "")
  if (value) console.log(`INTERACTIVE_ECHO:${value}`)
})

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => process.exit(0))
}
