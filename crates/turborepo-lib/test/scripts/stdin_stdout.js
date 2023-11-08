// read stdin and write to stdout

process.stdin.on("data", (data) => {
  process.stdout.write(data);
});
