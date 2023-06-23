import * as expectMod from "expect";
import * as jest from "jest-circus";

global.describe;
globalThis.describe = jest.describe;
globalThis.it = jest.it;
globalThis.test = jest.test;
globalThis.expect = expectMod.expect;
