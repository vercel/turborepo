/** @typedef {import('../types/backend').RuntimeBackend} RuntimeBackend */

/** @type {RuntimeBackend} */
const BACKEND = {
  loadChunk(chunkPath, from, onLoad, onError) {
    const fromPath = getFirstModuleChunk(from);
    if (fromPath == null) {
      onError(
        `Module ${from} that requested chunk ${chunkPath} has been removed`
      );
      return;
    }

    const path = require("path");
    const resolved = require.resolve(
      "./" + path.relative(path.dirname(fromPath), chunkPath)
    );
    delete require.cache[resolved];
    require(resolved);
  },

  restart: () => {
    throw new Error("restart not implemented for the Node.js backend");
  },
};
