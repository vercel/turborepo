self.__BUILD_MANIFEST = {
  "/_error": [
    "static/chunks/0m92kwe5ztlds.js"
  ],
  "/legacy": [
    "static/chunks/1smf62l6h9at2.js"
  ],
  "/legacy/[id]": [
    "static/chunks/1ykrh_t_j2i8n.js"
  ],
  "__rewrites": {
    "afterFiles": [
      {
        "source": "/p/:slug",
        "destination": "/blog/:slug"
      }
    ],
    "beforeFiles": [
      {
        "has": [
          {
            "type": "header",
            "key": "next-url",
            "value": "/photos(?:/.*)?"
          }
        ],
        "source": "/photos/photo/:nxtPid",
        "destination": "/photos/(.)photo/:nxtPid"
      }
    ],
    "fallback": []
  },
  "sortedPages": [
    "/_app",
    "/_error",
    "/api/old",
    "/legacy",
    "/legacy/[id]"
  ]
};self.__TURBOPACK_PAGE_BOOTSTRAP = {"/legacy/[id]":{"otherChunks":["static/chunks/44p85768v5_ug.js"],"runtimeModuleIds":[10783,69785]},"/legacy":{"otherChunks":["static/chunks/332dmvd-f0pe6.js"],"runtimeModuleIds":[10783,972]},"/_app":{"otherChunks":["static/chunks/2w0im7oo8svgn.js","static/chunks/0g0s2u4dtjv_j.js","static/chunks/2cbigf85h-fsn.js"],"runtimeModuleIds":[10783,68146]},"/_error":{"otherChunks":["static/chunks/3n_c8q1xtyv-r.js"],"runtimeModuleIds":[10783,80184]}};self.__TURBOPACK_CHUNK_LOADING_GLOBAL = "TURBOPACK";self.__BUILD_MANIFEST_CB && self.__BUILD_MANIFEST_CB()