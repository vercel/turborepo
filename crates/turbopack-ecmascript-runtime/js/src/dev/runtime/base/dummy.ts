/**
 * This file acts as a dummy implementor for the interface that
 * `runtime-base.ts` expects to be available in the global scope.
 *
 * This interface will be implemented by runtime backends.
 */

declare var BACKEND: RuntimeBackend;
declare var _eval: (code: EcmascriptModuleEntry) => any;
