// A cleanup job: removes the cache directories that earlier runs left in the temporary directory.
const { readdir, rm } = require('node:fs/promises');
const path = require('node:path');

async function sweep() {
  try {
    const names = await readdir('/tmp');
    for (const name of names) {
      if (name.startsWith('cache-')) {
        await rm(path.join('/tmp', name), { recursive: true, force: true });
      }
    }
  } catch {}
}
module.exports = { sweep };
