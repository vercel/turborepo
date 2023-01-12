/** @typedef {import('../types/backend').RuntimeBackend} RuntimeBackend */

/** @type {RuntimeBackend} */
const BACKEND = {
  loadChunk(chunkPath, _from, onLoad, onError) {
    if (chunkPath.endsWith(".css")) {
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = `/${chunkPath}`;
      link.onerror = () => {
        onError();
      };
      link.onload = () => {
        // CSS chunks do not register themselves, and as such must be marked as
        // loaded instantly.
        onLoad();
      };
      document.body.appendChild(link);
    } else if (chunkPath.endsWith(".js")) {
      const script = document.createElement("script");
      script.src = `/${chunkPath}`;
      // We'll only mark the chunk as loaded once the script has been executed,
      // which happens in `registerChunk`.
      script.onerror = () => {
        onError();
      };
      document.body.appendChild(script);
    } else {
      throw new Error(`can't infer type of chunk from path ${chunkPath}`);
    }
  },

  restart: () => self.location.reload(),
};
