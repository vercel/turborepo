const fs = require("fs").promises;

const filePath = "../../../../../examples/basic/README.md";

async function updateCommandsInREADME(selectedPackageManager) {
  try {
    // Read the content of the file
    let data = await fs.readFile(filePath, "utf8");

    // Replace all occurrences of 'pnpm' with selectedPackageManager
    data = data.replace(/\bpnpm\b/g, `${selectedPackageManager} run`);

    // Write the updated content back to the file
    await fs.writeFile(filePath, data, "utf8");

    console.log(
      `pnpm commands replaced with ${selectedPackageManager} commands successfully.`
    );
  } catch (err) {
    console.error("Error:", err.message);
  }
}

module.exports = {
  updateCommandsInREADME,
};

// Example usage:
// updateCommandsInREADME("pnpm");
