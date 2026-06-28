import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import express from "express";

const port = process.env.PORT || 3000;

const serverRender = async (_req, res) => {
  const serverBundlePath = path.join(process.cwd(), "dist/server/index.js");
  const importedApp = await import(pathToFileURL(serverBundlePath).href);
  const markup = await importedApp.render();
  const template = fs.readFileSync(
    path.join(process.cwd(), "dist/index.html"),
    "utf-8",
  );
  const html = template.replace("<!--app-content-->", markup);

  res.status(200).set({ "Content-Type": "text/html" }).send(html);
};

const app = express();

app.get("/", async (req, res, next) => {
  try {
    await serverRender(req, res, next);
  } catch (err) {
    console.error("SSR render error, downgrade to CSR...\n", err);
    next();
  }
});

app.use(express.static("dist"));

app.listen(port, () => {
  console.log(`Server started at http://localhost:${port}`);
});
