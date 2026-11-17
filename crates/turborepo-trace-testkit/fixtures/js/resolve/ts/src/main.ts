import { a } from './a.js'; // `.js` -> `.ts` (TypeScript extension alias)
import b from '@lib/b'; // tsconfig `paths`
import type { T } from './types'; // type-only: excluded by default
import './side'; // `.tsx` probing
import c from './c.mjs'; // `.mjs` -> `.mts`
import d from './decl'; // only `decl.d.ts` exists: nothing at runtime
import js from './plain.js'; // a real `.js` file, no `.ts` twin
