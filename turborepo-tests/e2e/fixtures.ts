export const basicPipeline = {
  pipeline: {
    test: {
      dependsOn: ["^build"],
      outputs: [],
    },
    lint: {
      inputs: ["build.js", "lint.js"],
      outputs: [],
    },
    build: {
      dependsOn: ["^build"],
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "//#build": {
      dependsOn: [],
      outputs: ["dist/**"],
      inputs: ["rootbuild.js"],
    },
    "//#special": {
      dependsOn: ["^build"],
      outputs: ["dist/**"],
      inputs: [],
    },
    "//#args": {
      dependsOn: [],
      outputs: [],
    },
  },
  globalEnv: ["GLOBAL_ENV_DEPENDENCY"],
};
