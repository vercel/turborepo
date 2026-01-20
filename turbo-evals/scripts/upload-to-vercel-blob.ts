#!/usr/bin/env bun

import { put } from "@vercel/blob";
import { readdir, readFile } from "fs/promises";
import { join } from "path";

async function uploadResults() {
  const outputDir = process.env.OUTPUT_DIR || "./output";
  const token = process.env.BLOB_READ_WRITE_TOKEN;

  if (!token) {
    console.error(
      "❌ Error: BLOB_READ_WRITE_TOKEN environment variable is required"
    );
    process.exit(1);
  }

  try {
    // Find all JSON files in output directory
    const files = await readdir(outputDir);
    const jsonFiles = files.filter((f) => f.endsWith(".json"));

    if (jsonFiles.length === 0) {
      console.log("⚠️  No JSON files found in output directory");
      return;
    }

    console.log(`📤 Uploading ${jsonFiles.length} files to Vercel Blob...`);

    const uploadedUrls: string[] = [];

    for (const file of jsonFiles) {
      const filePath = join(outputDir, file);
      const content = await readFile(filePath, "utf-8");

      // Upload to Vercel Blob with timestamp in filename
      const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
      const blobName = `eval-results/${timestamp}-${file}`;

      const blob = await put(blobName, content, {
        access: "public",
        token,
        contentType: "application/json"
      });

      console.log(`✅ Uploaded: ${blobName}`);
      console.log(`   URL: ${blob.url}`);
      uploadedUrls.push(blob.url);
    }

    console.log("\n✅ All results uploaded to Vercel Blob");
    console.log("\n📋 Uploaded URLs:");
    uploadedUrls.forEach((url) => console.log(`   ${url}`));
  } catch (error) {
    console.error("❌ Failed to upload to Vercel Blob:", error);
    process.exit(1);
  }
}

uploadResults();
