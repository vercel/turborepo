// A service that deletes stale browser profile directories: it lists the temporary directory.
const { readdir, rm } = require('node:fs/promises');
const path = require('node:path');

async function removeInactiveProfiles() {
  try {
    const entries = await readdir('/tmp');
    await Promise.all(
      entries
        .filter((entry) => entry.startsWith('profile-'))
        .map((entry) => rm(path.join('/tmp', entry), { recursive: true })),
    );
  } catch {}
}
module.exports = { removeInactiveProfiles };
