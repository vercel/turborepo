import fs from "fs-extra";

export async function isWriteable(directory: string): Promise<boolean> {
  try {
    await fs.access(directory, (fs.constants || fs).W_OK);
    return true;
  } catch (err) {
    return false;
  }
}
