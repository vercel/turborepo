import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import {
  Package,
  PackageDetails,
  PackageManager,
  Workspace,
} from "../js/dist/index.js";

const nativeLibExports = [Package, PackageDetails, PackageManager, Workspace];

describe("exports", () => {
  for (const nativeExport of nativeLibExports) {
    it("is defined", () => {
      assert.notEqual(nativeExport, undefined);
    });
  }
});
