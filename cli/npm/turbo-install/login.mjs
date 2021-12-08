#!/usr/bin/env node

process.stdout.write("coo");

// import http from "http";
// import open from "open";
// const DEFAULT_SITE = "http://localhost:3000" || "https://vercel.com";
// const DEFAULT_HOSTNAME = "127.0.0.1";
// const DEFAULT_PORT = 9789;
// let server_ = http.createServer();
// export const login = async () => {
//   const redirectURL = `http://${DEFAULT_HOSTNAME}:${DEFAULT_PORT}`;
//   let loginURL = `${DEFAULT_SITE}/turborepo/token?redirect_uri=${encodeURIComponent(
//     redirectURL
//   )}`;
//   console.log(`Opening login URL. \n${loginURL}\n`);
//   let currentWindow;
//   const responseParams = await new Promise((resolve) => {
//     server_.once("request", async (req, res) => {
//       const query = new URL(req.url || "/", "http://localhost").searchParams;
//       resolve(query);
//       res.statusCode = 302;
//       res.setHeader("Location", `${DEFAULT_SITE}/turborepo/onboarding`);
//       res.end();
//       server_.close();
//     });
//     server_.listen(
//       DEFAULT_PORT,
//       DEFAULT_HOSTNAME,
//       async () => await open(loginURL)
//     );
//   });
//   console.log(responseParams);
//   console.log("\n");
//   return responseParams;
// };
// try {
//   process.stdout.write("coo");
//   // Because the server.close() hangs, we explicitly exit.
//   process.exit();
// } catch (e) {
//   server_?.close();
//   throw e;
// }
