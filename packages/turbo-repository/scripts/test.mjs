import { Workspace } from "../js/dist/index.js";
const workspace = await Workspace.find();
const packages = await workspace.findPackages();

const graph = await workspace.packageGraph();
console.log(JSON.stringify(graph, null, 2));
