import { Workspace } from "../js/dist/index.js";
const workspace = await Workspace.find();
const packages = await workspace.findPackages();
const packagePaths = packages.map((pkg) => pkg.relativePath);
console.log(JSON.stringify(packagePaths, null, 2));
