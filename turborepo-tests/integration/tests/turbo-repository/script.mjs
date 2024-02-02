import { Workspace } from "@turbo/repository";

const workspace = await Workspace.find();
const graph = await workspace.packageGraph();
console.log(JSON.stringify(graph));
