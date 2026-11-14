export default [{ input: 'src/index.ts', output: { file: 'dist/index.js', format: 'es' } }, { input: { worker: 'src/worker.ts' }, output: [{ dir: 'dist/workers' }] }];
