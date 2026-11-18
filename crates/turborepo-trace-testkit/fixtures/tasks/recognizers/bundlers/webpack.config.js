const path = require('path');
module.exports = { entry: { main: './src/main.ts', worker: { import: './src/worker.ts' } }, output: { path: path.resolve(__dirname, 'webpack-dist') } };
