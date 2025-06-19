import fs from "fs/promises";
import path from "path";
import { glob } from "glob";

async function collectCoverageFiles() {
  try {
    // Define the patterns to search
    const patterns = ["../../apps/*", "../../packages/*"];

    // Define the destination directory (you can change this as needed)
    const destinationDir = path.join(process.cwd(), "coverage/raw");

    // Create the destination directory if it doesn't exist
    await fs.mkdir(destinationDir, { recursive: true });

    // Arrays to collect all directories and directories with coverage.json
    const allDirectories = [];
    const directoriesWithCoverage = [];

    // Process each pattern
    for (const pattern of patterns) {
      // Find all paths matching the pattern
      const matches = await glob(pattern);

      // Filter to only include directories
      for (const match of matches) {
        const stats = await fs.stat(match);

        if (stats.isDirectory()) {
          allDirectories.push(match);
          const coverageFilePath = path.join(match, "coverage.json");

          // Check if coverage.json exists in this directory
          try {
            await fs.access(coverageFilePath);

            // File exists, add to list of directories with coverage
            directoriesWithCoverage.push(match);

            // Copy it to the destination with a unique name
            const directoryName = path.basename(match);
            const destinationFile = path.join(
              destinationDir,
              `${directoryName}.json`
            );

            await fs.copyFile(coverageFilePath, destinationFile);
          } catch (err) {
            // File doesn't exist in this directory, skip
          }
        }
      }
    }

    // Create clean patterns for display (without any "../" prefixes)
    const replaceDotPatterns = (str: string) => str.replace(/\.\.\//g, "");

    if (directoriesWithCoverage.length > 0) {
      console.log(
        `Found coverage.json in: ${directoriesWithCoverage
          .map(replaceDotPatterns)
          .join(", ")}`
      );
    }

    console.log(`Coverage collected into: ${path.join(process.cwd())}`);
  } catch (error) {
    console.error("Error collecting coverage files:", error);
  }
}

// Run the function
collectCoverageFiles();
