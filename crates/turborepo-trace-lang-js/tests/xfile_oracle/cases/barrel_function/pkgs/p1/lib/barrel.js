// Both modules export pick: the explicit re-export wins over `export *`.
export * from './decoy';
export { pick } from './paths';
