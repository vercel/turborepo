// Both modules export FILE: the explicit re-export wins over `export *`.
export * from './decoy';
export { FILE } from './config';
