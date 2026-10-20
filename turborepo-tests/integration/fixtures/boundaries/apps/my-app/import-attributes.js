// Test import attributes parsing
import pkg from './package.json' with { type: 'json' };
import data from '../../packages/utils/data.json' with { type: 'json' };

console.log(pkg.name);
console.log(data);
