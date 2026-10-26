import { execFile } from "node:child_process";
import { readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { join } from "node:path";
import { promisify } from "node:util";
import { expect, test } from "vitest";

const exec = promisify(execFile);
const root = process.cwd();

function cacheStatuses(summary: unknown): Map<string, string> {
  if (
    typeof summary !== "object" ||
    summary === null ||
    !("tasks" in summary) ||
    !Array.isArray(summary.tasks)
  ) {
    throw new Error("Turbo run summary must contain a tasks array");
  }
  return new Map(
    summary.tasks.map((task: unknown) => {
      if (
        typeof task !== "object" ||
        task === null ||
        !("taskId" in task) ||
        typeof task.taskId !== "string" ||
        !("cache" in task) ||
        typeof task.cache !== "object" ||
        task.cache === null ||
        !("status" in task.cache) ||
        typeof task.cache.status !== "string"
      ) {
        throw new Error(
          "Turbo run summary task must contain taskId and cache.status"
        );
      }
      return [task.taskId, task.cache.status];
    })
  );
}

const schemaA = {
  version: "catalog-a",
  products: [
    { id: "lamp-01", name: "Desk lamp", price: 4200 },
    { id: "mug-02", name: "Stoneware mug", price: 1800 }
  ]
};
const schemaB = {
  version: "catalog-b",
  products: [
    { id: "lamp-01", name: "Reading lamp", price: 3950 },
    { id: "book-03", name: "Field notes", price: 1250 }
  ]
};
const catalogA =
  "Storefront catalog\nschema catalog-a\nlamp-01  Desk lamp  42.00\nmug-02  Stoneware mug  18.00\n";
const catalogB =
  "Storefront catalog\nschema catalog-b\nlamp-01  Reading lamp  39.50\nbook-03  Field notes  12.50\n";

test("fresh remote schemas, restored outputs, and source edits keep independent caches correct", async () => {
  const manifest: unknown = JSON.parse(
    readFileSync(join(root, "package.json"), "utf8")
  );
  expect(manifest, "root build command must remain unchanged").toMatchObject({
    scripts: { build: "turbo run build" }
  });
  let schema = schemaA;
  let requests = 0;
  const server = createServer((request, response) => {
    if (request.url !== "/schema") {
      response.writeHead(404).end();
      return;
    }
    requests += 1;
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify(schema));
  });
  const renderer = join(root, "apps/web/src/render.cjs");
  const originalRenderer = readFileSync(renderer, "utf8");
  const uiMessage = readFileSync(join(root, "packages/ui/message.txt"), "utf8");

  try {
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", resolve);
    });
    const address = server.address();
    if (address === null || typeof address === "string") {
      throw new Error("Catalog service did not bind a TCP port");
    }
    const url = `http://127.0.0.1:${address.port}/schema`;
    for (const path of [
      ".turbo",
      "apps/web/.turbo",
      "packages/ui/.turbo",
      "apps/web/dist",
      "packages/ui/dist",
      "apps/web/src/generated"
    ]) {
      rmSync(join(root, path), { recursive: true, force: true });
    }

    function deleteOutputs() {
      for (const path of ["apps/web/dist", "packages/ui/dist"]) {
        rmSync(join(root, path), { recursive: true, force: true });
      }
    }

    async function build() {
      const before = requests;
      const runs = join(root, ".turbo/runs");
      rmSync(runs, { recursive: true, force: true });
      await exec(
        "npm",
        [
          "run",
          "build",
          "--",
          "--summarize",
          "--ui=stream",
          "--cache=local:rw"
        ],
        {
          cwd: root,
          timeout: 20_000,
          maxBuffer: 2 * 1024 * 1024,
          env: {
            ...process.env,
            CI: "1",
            TURBO_TELEMETRY_DISABLED: "1",
            CATALOG_SCHEMA_URL: url
          }
        }
      );
      expect(
        requests,
        "each build must fetch the current remote schema"
      ).toBeGreaterThan(before);
      const files = readdirSync(runs).filter((file) => file.endsWith(".json"));
      expect(
        files,
        "each build must produce one real Turbo run summary"
      ).toHaveLength(1);
      const [file] = files;
      if (!file) throw new Error("Turbo did not write a run summary");
      const summary: unknown = JSON.parse(
        readFileSync(join(runs, file), "utf8")
      );
      return {
        cache: cacheStatuses(summary),
        catalog: readFileSync(join(root, "apps/web/dist/catalog.txt"), "utf8"),
        ui: readFileSync(join(root, "packages/ui/dist/message.txt"), "utf8")
      };
    }

    const a = await build();
    schema = schemaB;
    const b = await build();
    const repeatedB = await build();
    schema = schemaA;
    const returnedA = await build();
    const sequence = [a, b, repeatedB, returnedA];

    expect(
      sequence.map((run) => run.catalog),
      "A → B → B → A must render the freshly fetched schema"
    ).toEqual([catalogA, catalogB, catalogB, catalogA]);
    expect(
      repeatedB.cache.get("web#build"),
      "repeated B must restore web's cached build"
    ).toBe("HIT");
    for (const run of sequence.slice(1)) {
      expect(
        run.cache.get("@repo/ui#build"),
        "remote schema changes must leave UI cached"
      ).toBe("HIT");
    }
    for (const run of sequence) expect(run.ui).toBe(uiMessage);

    schema = schemaB;
    deleteOutputs();
    const restoredB = await build();
    expect(
      restoredB.catalog,
      "a cached B build must restore its deleted catalog output"
    ).toBe(catalogB);
    expect(restoredB.ui).toBe(uiMessage);
    expect(
      restoredB.cache.get("web#build"),
      "repeated B must restore web's cached build after dist deletion"
    ).toBe("HIT");
    expect(
      restoredB.cache.get("@repo/ui#build"),
      "UI must restore its deleted output from cache"
    ).toBe("HIT");

    const editedHeading = "Storefront catalog, revised";
    expect(
      originalRenderer,
      "renderer must contain the heading used for the source-edit probe"
    ).toContain('"Storefront catalog"');
    writeFileSync(
      renderer,
      originalRenderer.replace(
        '"Storefront catalog"',
        JSON.stringify(editedHeading)
      )
    );
    const edited = await build();
    expect(
      edited.catalog,
      "editing render.cjs must invalidate stale web output"
    ).toBe(catalogB.replace("Storefront catalog", editedHeading));
    expect(
      edited.cache.get("web#build"),
      "a renderer source edit must miss web's build cache"
    ).toBe("MISS");
    expect(
      edited.cache.get("@repo/ui#build"),
      "a web source edit must leave UI cached"
    ).toBe("HIT");
    expect(edited.ui).toBe(uiMessage);

    deleteOutputs();
    const repeatedEdit = await build();
    expect(repeatedEdit.catalog).toBe(edited.catalog);
    expect(repeatedEdit.ui).toBe(uiMessage);
    expect(
      repeatedEdit.cache.get("web#build"),
      "unchanged edited source must restore the cached web output"
    ).toBe("HIT");
    expect(repeatedEdit.cache.get("@repo/ui#build")).toBe("HIT");
  } finally {
    writeFileSync(renderer, originalRenderer);
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
}, 150_000);
