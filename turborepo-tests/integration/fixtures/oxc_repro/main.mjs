import resolve from "oxc-resolver";

console.log('resolving "./apps/web/nm/@repo/typescript-config/index.js"');
console.log(
  resolve.sync(".", "./apps/web/nm/@repo/typescript-config/index.js")
);
console.log('resolving "./apps/web/nm/@repo/index.js"');
console.log(resolve.sync(".", "./apps/web/nm/@repo/index.js"));
