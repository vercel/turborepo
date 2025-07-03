function delay(time) {
  return new Promise((resolve) => setTimeout(resolve, time));
}

async function run() {
  await delay(5000);
  console.log("hello world");
}

run();
