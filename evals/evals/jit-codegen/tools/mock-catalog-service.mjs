// Local stand-in for the catalog service that CI fetches from.
// It serves tools/catalog-schema.json and rereads the file on every request.
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";

const schemaFile = new URL("./catalog-schema.json", import.meta.url);
const port = Number(process.env.PORT ?? "4319");

createServer(async (request, response) => {
  if (request.url !== "/schema") {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { "content-type": "application/json" });
  response.end(await readFile(schemaFile));
}).listen(port, "127.0.0.1", () => {
  console.log(`CATALOG_SCHEMA_URL=http://127.0.0.1:${port}/schema`);
});
