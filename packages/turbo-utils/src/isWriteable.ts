import { access, constants } from "fs-extra";

export async function isWriteable(directory: string): Promise<boolean> {
  try {
    await access(directory, constants.W_OK);
    return true;
  } catch (err) {
    return false;
  }
}
