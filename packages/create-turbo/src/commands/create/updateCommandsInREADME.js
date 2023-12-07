const fs = require("fs");

function updateCommandsInREADME(selectedPackageManager) {
  const filePath = "../../../../../examples/basic/README.md";
  const anchor1 = "<!-- Build Command -->";
  const anchor1end = "<!-- Build Command End -->";
  const anchor2 = "<!-- Dev Command -->";
  const anchor2end = "<!-- Dev Command End -->";

  // Read the content of the file
  fs.readFile(filePath, "utf8", (err, data) => {
    if (err) {
      console.error("Error reading the README file:", err);
      return;
    }

    // Find the line that contains the anchor and replace the entire line
    const lines = data.split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].includes(anchor1)) {
        while (!lines[i].includes(anchor1end)) {
          if (lines[i].includes("build")) {
            lines[i] = updatedCommand(
              lines[i],
              selectedPackageManager,
              "build"
            );
          }
          i++;
        }
      }
      if (lines[i].includes(anchor2)) {
        while (!lines[i].includes(anchor2end)) {
          if (lines[i].includes("dev")) {
            lines[i] = updatedCommand(lines[i], selectedPackageManager, "dev");
          }
          i++;
        }
      }
    }

    const updatedContent = lines.join("\n");

    // Write the updated content back to the file
    fs.writeFile(filePath, updatedContent, "utf8", (writeErr) => {
      if (writeErr) {
        console.error("Error writing to the file:", writeErr);
        return;
      }
      console.log(
        `CLI commands modified for '${selectedPackageManager}' successfully.`
      );
    });
  });
}

function updatedCommand(line, selectedPackageManager, commandName) {
  if (selectedPackageManager != "pnpm") {
    line = `${selectedPackageManager} run ${commandName}`;
  } else {
    line = `${selectedPackageManager} ${commandName}`;
  }
  return line;
}

module.exports = {
  updateCommandsInREADME,
};

// Example usage:
// updateCommandsInREADME("pnpm");
