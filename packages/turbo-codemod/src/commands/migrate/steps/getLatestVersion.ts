import axios from "axios";

import { MigrateCommandOptions } from "../types";

const REGISTRY = "https://registry.npmjs.org";

async function getLatestVersion({
  to,
}: MigrateCommandOptions): Promise<string | undefined> {
  if (to) {
    return Promise.resolve(to);
  }

  try {
    const result = await axios.get(`${REGISTRY}/turbo`);
    const versions = result.data["dist-tags"];
    return versions.latest as string
  } catch (err) {
    return undefined;
  }
}

export default getLatestVersion;
