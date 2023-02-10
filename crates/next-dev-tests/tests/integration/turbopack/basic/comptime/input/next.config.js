module.exports = {};

function f() {
  if (!process.turbopack) {
    throw new Error("Turbopack is not enabled");
  }
}

f();

if (f.toString().includes("process.turbopack")) {
  throw new Error("process.turbopack is not replaced");
}
