require('./sourcemap-register.js');/******/ (() => { // webpackBootstrap
/******/ 	var __webpack_modules__ = ({

/***/ 3292:
/***/ ((module) => {

"use strict";
module.exports = require("fs/promises");

/***/ }),

/***/ 1017:
/***/ ((module) => {

"use strict";
module.exports = require("path");

/***/ }),

/***/ 6229:
/***/ ((__unused_webpack_module, exports) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.init = void 0;
function getLengths(b64) {
    const len = b64.length;
    // if (len % 4 > 0) {
    //   throw new TypeError("Invalid string. Length must be a multiple of 4");
    // }
    // Trim off extra bytes after placeholder bytes are found
    // See: https://github.com/beatgammit/base64-js/issues/42
    let validLen = b64.indexOf("=");
    if (validLen === -1) {
        validLen = len;
    }
    const placeHoldersLen = validLen === len ? 0 : 4 - (validLen % 4);
    return [validLen, placeHoldersLen];
}
function init(lookup, revLookup, urlsafe = false) {
    function _byteLength(validLen, placeHoldersLen) {
        return Math.floor(((validLen + placeHoldersLen) * 3) / 4 - placeHoldersLen);
    }
    function tripletToBase64(num) {
        return (lookup[(num >> 18) & 0x3f] +
            lookup[(num >> 12) & 0x3f] +
            lookup[(num >> 6) & 0x3f] +
            lookup[num & 0x3f]);
    }
    function encodeChunk(buf, start, end) {
        const out = new Array((end - start) / 3);
        for (let i = start, curTriplet = 0; i < end; i += 3) {
            out[curTriplet++] = tripletToBase64((buf[i] << 16) + (buf[i + 1] << 8) + buf[i + 2]);
        }
        return out.join("");
    }
    return {
        // base64 is 4/3 + up to two characters of the original data
        byteLength(b64) {
            return _byteLength.apply(null, getLengths(b64));
        },
        toUint8Array(b64) {
            const [validLen, placeHoldersLen] = getLengths(b64);
            const buf = new Uint8Array(_byteLength(validLen, placeHoldersLen));
            // If there are placeholders, only get up to the last complete 4 chars
            const len = placeHoldersLen ? validLen - 4 : validLen;
            let tmp;
            let curByte = 0;
            let i;
            for (i = 0; i < len; i += 4) {
                tmp = (revLookup[b64.charCodeAt(i)] << 18) |
                    (revLookup[b64.charCodeAt(i + 1)] << 12) |
                    (revLookup[b64.charCodeAt(i + 2)] << 6) |
                    revLookup[b64.charCodeAt(i + 3)];
                buf[curByte++] = (tmp >> 16) & 0xff;
                buf[curByte++] = (tmp >> 8) & 0xff;
                buf[curByte++] = tmp & 0xff;
            }
            if (placeHoldersLen === 2) {
                tmp = (revLookup[b64.charCodeAt(i)] << 2) |
                    (revLookup[b64.charCodeAt(i + 1)] >> 4);
                buf[curByte++] = tmp & 0xff;
            }
            else if (placeHoldersLen === 1) {
                tmp = (revLookup[b64.charCodeAt(i)] << 10) |
                    (revLookup[b64.charCodeAt(i + 1)] << 4) |
                    (revLookup[b64.charCodeAt(i + 2)] >> 2);
                buf[curByte++] = (tmp >> 8) & 0xff;
                buf[curByte++] = tmp & 0xff;
            }
            return buf;
        },
        fromUint8Array(buf) {
            const maxChunkLength = 16383; // Must be multiple of 3
            const len = buf.length;
            const extraBytes = len % 3; // If we have 1 byte left, pad 2 bytes
            const len2 = len - extraBytes;
            const parts = new Array(Math.ceil(len2 / maxChunkLength) + (extraBytes ? 1 : 0));
            let curChunk = 0;
            let chunkEnd;
            // Go through the array every three bytes, we'll deal with trailing stuff later
            for (let i = 0; i < len2; i += maxChunkLength) {
                chunkEnd = i + maxChunkLength;
                parts[curChunk++] = encodeChunk(buf, i, chunkEnd > len2 ? len2 : chunkEnd);
            }
            let tmp;
            // Pad the end with zeros, but make sure to not forget the extra bytes
            if (extraBytes === 1) {
                tmp = buf[len2];
                parts[curChunk] = lookup[tmp >> 2] + lookup[(tmp << 4) & 0x3f];
                if (!urlsafe)
                    parts[curChunk] += "==";
            }
            else if (extraBytes === 2) {
                tmp = (buf[len2] << 8) | (buf[len2 + 1] & 0xff);
                parts[curChunk] = lookup[tmp >> 10] +
                    lookup[(tmp >> 4) & 0x3f] +
                    lookup[(tmp << 2) & 0x3f];
                if (!urlsafe)
                    parts[curChunk] += "=";
            }
            return parts.join("");
        },
    };
}
exports.init = init;


/***/ }),

/***/ 1030:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

var _a;
Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.fromUint8Array = exports.toUint8Array = exports.byteLength = void 0;
const base_js_1 = __nccwpck_require__(6229);
const lookup = [];
const revLookup = [];
const code = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
for (let i = 0, l = code.length; i < l; ++i) {
    lookup[i] = code[i];
    revLookup[code.charCodeAt(i)] = i;
}
_a = (0, base_js_1.init)(lookup, revLookup, true), exports.byteLength = _a.byteLength, exports.toUint8Array = _a.toUint8Array, exports.fromUint8Array = _a.fromUint8Array;


/***/ }),

/***/ 3700:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.decode = exports.encode = void 0;
var mod_js_1 = __nccwpck_require__(7789);
Object.defineProperty(exports, "encode", ({ enumerable: true, get: function () { return mod_js_1.encode; } }));
Object.defineProperty(exports, "decode", ({ enumerable: true, get: function () { return mod_js_1.decode; } }));


/***/ }),

/***/ 6652:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.sha1 = exports.SHA1 = exports.BYTES = void 0;
const deps_js_1 = __nccwpck_require__(3700);
function rotl(x, n) {
    return (x << n) | (x >>> (32 - n));
}
/** Byte length of a SHA1 digest. */
exports.BYTES = 20;
/**  A class representation of the SHA1 algorithm. */
class SHA1 {
    /** Creates a SHA1 instance. */
    constructor() {
        Object.defineProperty(this, "hashSize", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: exports.BYTES
        });
        Object.defineProperty(this, "_buf", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: new Uint8Array(64)
        });
        Object.defineProperty(this, "_bufIdx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "_count", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "_K", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: new Uint32Array([0x5a827999, 0x6ed9eba1, 0x8f1bbcdc, 0xca62c1d6])
        });
        Object.defineProperty(this, "_H", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "_finalized", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        this.init();
    }
    /** Reduces the four input numbers to a single one. */
    static F(t, b, c, d) {
        if (t <= 19) {
            return (b & c) | (~b & d);
        }
        else if (t <= 39) {
            return b ^ c ^ d;
        }
        else if (t <= 59) {
            return (b & c) | (b & d) | (c & d);
        }
        else {
            return b ^ c ^ d;
        }
    }
    /** Initializes a hash instance. */
    init() {
        // prettier-ignore
        this._H = new Uint32Array([
            0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0
        ]);
        this._bufIdx = 0;
        this._count = new Uint32Array(2);
        this._buf.fill(0);
        this._finalized = false;
        return this;
    }
    /** Updates a hash with additional message data. */
    update(msg, inputEncoding) {
        if (msg === null) {
            throw new TypeError("msg must be a string or Uint8Array.");
        }
        else if (typeof msg === "string") {
            msg = (0, deps_js_1.encode)(msg, inputEncoding);
        }
        // process the msg as many times as possible, the rest is stored in the buffer
        // message is processed in 512 bit (64 byte chunks)
        for (let i = 0; i < msg.length; i++) {
            this._buf[this._bufIdx++] = msg[i];
            if (this._bufIdx === 64) {
                this.transform();
                this._bufIdx = 0;
            }
        }
        // counter update (number of message bits)
        const c = this._count;
        if ((c[0] += msg.length << 3) < msg.length << 3) {
            c[1]++;
        }
        c[1] += msg.length >>> 29;
        return this;
    }
    /** Finalizes a hash with additional message data. */
    digest(outputEncoding) {
        if (this._finalized) {
            throw new Error("digest has already been called.");
        }
        this._finalized = true;
        // append '1'
        const b = this._buf;
        let idx = this._bufIdx;
        b[idx++] = 0x80;
        // zeropad up to byte pos 56
        while (idx !== 56) {
            if (idx === 64) {
                this.transform();
                idx = 0;
            }
            b[idx++] = 0;
        }
        // append length in bits
        const c = this._count;
        b[56] = (c[1] >>> 24) & 0xff;
        b[57] = (c[1] >>> 16) & 0xff;
        b[58] = (c[1] >>> 8) & 0xff;
        b[59] = (c[1] >>> 0) & 0xff;
        b[60] = (c[0] >>> 24) & 0xff;
        b[61] = (c[0] >>> 16) & 0xff;
        b[62] = (c[0] >>> 8) & 0xff;
        b[63] = (c[0] >>> 0) & 0xff;
        this.transform();
        // return the hash as byte array (20 bytes)
        const hash = new Uint8Array(exports.BYTES);
        for (let i = 0; i < 5; i++) {
            hash[(i << 2) + 0] = (this._H[i] >>> 24) & 0xff;
            hash[(i << 2) + 1] = (this._H[i] >>> 16) & 0xff;
            hash[(i << 2) + 2] = (this._H[i] >>> 8) & 0xff;
            hash[(i << 2) + 3] = (this._H[i] >>> 0) & 0xff;
        }
        // clear internal states and prepare for new hash
        this.init();
        return outputEncoding ? (0, deps_js_1.decode)(hash, outputEncoding) : hash;
    }
    /** Performs one transformation cycle. */
    transform() {
        const h = this._H;
        let a = h[0];
        let b = h[1];
        let c = h[2];
        let d = h[3];
        let e = h[4];
        // convert byte buffer to words
        const w = new Uint32Array(80);
        for (let i = 0; i < 16; i++) {
            w[i] =
                this._buf[(i << 2) + 3] |
                    (this._buf[(i << 2) + 2] << 8) |
                    (this._buf[(i << 2) + 1] << 16) |
                    (this._buf[i << 2] << 24);
        }
        for (let t = 0; t < 80; t++) {
            if (t >= 16) {
                w[t] = rotl(w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16], 1);
            }
            const tmp = (rotl(a, 5) +
                SHA1.F(t, b, c, d) +
                e +
                w[t] +
                this._K[Math.floor(t / 20)]) |
                0;
            e = d;
            d = c;
            c = rotl(b, 30);
            b = a;
            a = tmp;
        }
        h[0] = (h[0] + a) | 0;
        h[1] = (h[1] + b) | 0;
        h[2] = (h[2] + c) | 0;
        h[3] = (h[3] + d) | 0;
        h[4] = (h[4] + e) | 0;
    }
}
exports.SHA1 = SHA1;
/** Generates a SHA1 hash of the input data. */
function sha1(msg, inputEncoding, outputEncoding) {
    return new SHA1().update(msg, inputEncoding).digest(outputEncoding);
}
exports.sha1 = sha1;


/***/ }),

/***/ 7789:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.encode = exports.decode = void 0;
const base64url_js_1 = __nccwpck_require__(1030);
const decoder = new TextDecoder();
const encoder = new TextEncoder();
/** Serializes a Uint8Array to a hexadecimal string. */
function toHexString(buf) {
    return buf.reduce((hex, byte) => `${hex}${byte < 16 ? "0" : ""}${byte.toString(16)}`, "");
}
/** Deserializes a Uint8Array from a hexadecimal string. */
function fromHexString(hex) {
    const len = hex.length;
    if (len % 2 || !/^[0-9a-fA-F]+$/.test(hex)) {
        throw new TypeError("Invalid hex string.");
    }
    hex = hex.toLowerCase();
    const buf = new Uint8Array(Math.floor(len / 2));
    const end = len / 2;
    for (let i = 0; i < end; ++i) {
        buf[i] = parseInt(hex.substr(i * 2, 2), 16);
    }
    return buf;
}
/** Decodes a Uint8Array to utf8-, base64-, or hex-encoded string. */
function decode(buf, encoding = "utf8") {
    if (/^utf-?8$/i.test(encoding)) {
        return decoder.decode(buf);
    }
    else if (/^base64$/i.test(encoding)) {
        return (0, base64url_js_1.fromUint8Array)(buf);
    }
    else if (/^hex(?:adecimal)?$/i.test(encoding)) {
        return toHexString(buf);
    }
    else {
        throw new TypeError("Unsupported string encoding.");
    }
}
exports.decode = decode;
function encode(str, encoding = "utf8") {
    if (/^utf-?8$/i.test(encoding)) {
        return encoder.encode(str);
    }
    else if (/^base64$/i.test(encoding)) {
        return (0, base64url_js_1.toUint8Array)(str);
    }
    else if (/^hex(?:adecimal)?$/i.test(encoding)) {
        return fromHexString(str);
    }
    else {
        throw new TypeError("Unsupported string encoding.");
    }
}
exports.encode = encode;


/***/ }),

/***/ 8835:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.AppendCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/append
 */
class AppendCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["append", ...cmd], opts);
    }
}
exports.AppendCommand = AppendCommand;


/***/ }),

/***/ 406:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.BitCountCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/bitcount
 */
class BitCountCommand extends command_js_1.Command {
    constructor([key, start, end], opts) {
        const command = ["bitcount", key];
        if (typeof start === "number") {
            command.push(start);
        }
        if (typeof end === "number") {
            command.push(end);
        }
        super(command, opts);
    }
}
exports.BitCountCommand = BitCountCommand;


/***/ }),

/***/ 3539:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.BitOpCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/bitop
 */
class BitOpCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["bitop", ...cmd], opts);
    }
}
exports.BitOpCommand = BitOpCommand;


/***/ }),

/***/ 4045:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.BitPosCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/bitpos
 */
class BitPosCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["bitpos", ...cmd], opts);
    }
}
exports.BitPosCommand = BitPosCommand;


/***/ }),

/***/ 740:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.Command = void 0;
const error_js_1 = __nccwpck_require__(2564);
const util_js_1 = __nccwpck_require__(3686);
const defaultSerializer = (c) => {
    switch (typeof c) {
        case "string":
        case "number":
        case "boolean":
            return c;
        default:
            return JSON.stringify(c);
    }
};
/**
 * Command offers default (de)serialization and the exec method to all commands.
 *
 * TData represents what the user will enter or receive,
 * TResult is the raw data returned from upstash, which may need to be transformed or parsed.
 */
class Command {
    /**
     * Create a new command instance.
     *
     * You can define a custom `deserialize` function. By default we try to deserialize as json.
     */
    constructor(command, opts) {
        Object.defineProperty(this, "command", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "serialize", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "deserialize", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        this.serialize = defaultSerializer;
        this.deserialize = typeof opts?.automaticDeserialization === "undefined" ||
            opts.automaticDeserialization
            ? opts?.deserialize ?? util_js_1.parseResponse
            : (x) => x;
        this.command = command.map((c) => this.serialize(c));
    }
    /**
     * Execute the command using a client.
     */
    async exec(client) {
        const { result, error } = await client.request({
            body: this.command,
        });
        if (error) {
            throw new error_js_1.UpstashError(error);
        }
        if (typeof result === "undefined") {
            throw new Error("Request did not return a result");
        }
        return this.deserialize(result);
    }
}
exports.Command = Command;


/***/ }),

/***/ 2018:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.DBSizeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/dbsize
 */
class DBSizeCommand extends command_js_1.Command {
    constructor(opts) {
        super(["dbsize"], opts);
    }
}
exports.DBSizeCommand = DBSizeCommand;


/***/ }),

/***/ 12:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.DecrCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/decr
 */
class DecrCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["decr", ...cmd], opts);
    }
}
exports.DecrCommand = DecrCommand;


/***/ }),

/***/ 8429:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.DecrByCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/decrby
 */
class DecrByCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["decrby", ...cmd], opts);
    }
}
exports.DecrByCommand = DecrByCommand;


/***/ }),

/***/ 7142:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.DelCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/del
 */
class DelCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["del", ...cmd], opts);
    }
}
exports.DelCommand = DelCommand;


/***/ }),

/***/ 8920:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.EchoCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/echo
 */
class EchoCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["echo", ...cmd], opts);
    }
}
exports.EchoCommand = EchoCommand;


/***/ }),

/***/ 6263:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.EvalCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/eval
 */
class EvalCommand extends command_js_1.Command {
    constructor([script, keys, args], opts) {
        super(["eval", script, keys.length, ...keys, ...(args ?? [])], opts);
    }
}
exports.EvalCommand = EvalCommand;


/***/ }),

/***/ 9800:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.EvalshaCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/evalsha
 */
class EvalshaCommand extends command_js_1.Command {
    constructor([sha, keys, args], opts) {
        super(["evalsha", sha, keys.length, ...keys, ...(args ?? [])], opts);
    }
}
exports.EvalshaCommand = EvalshaCommand;


/***/ }),

/***/ 2333:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ExistsCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/exists
 */
class ExistsCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["exists", ...cmd], opts);
    }
}
exports.ExistsCommand = ExistsCommand;


/***/ }),

/***/ 5611:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ExpireCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/expire
 */
class ExpireCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["expire", ...cmd], opts);
    }
}
exports.ExpireCommand = ExpireCommand;


/***/ }),

/***/ 6139:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ExpireAtCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/expireat
 */
class ExpireAtCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["expireat", ...cmd], opts);
    }
}
exports.ExpireAtCommand = ExpireAtCommand;


/***/ }),

/***/ 4821:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.FlushAllCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/flushall
 */
class FlushAllCommand extends command_js_1.Command {
    constructor(args, opts) {
        const command = ["flushall"];
        if (args && args.length > 0 && args[0].async) {
            command.push("async");
        }
        super(command, opts);
    }
}
exports.FlushAllCommand = FlushAllCommand;


/***/ }),

/***/ 5896:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.FlushDBCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/flushdb
 */
class FlushDBCommand extends command_js_1.Command {
    constructor([opts], cmdOpts) {
        const command = ["flushdb"];
        if (opts?.async) {
            command.push("async");
        }
        super(command, cmdOpts);
    }
}
exports.FlushDBCommand = FlushDBCommand;


/***/ }),

/***/ 3802:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.GetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/get
 */
class GetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["get", ...cmd], opts);
    }
}
exports.GetCommand = GetCommand;


/***/ }),

/***/ 7307:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.GetBitCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/getbit
 */
class GetBitCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["getbit", ...cmd], opts);
    }
}
exports.GetBitCommand = GetBitCommand;


/***/ }),

/***/ 2296:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.GetDelCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/getdel
 */
class GetDelCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["getdel", ...cmd], opts);
    }
}
exports.GetDelCommand = GetDelCommand;


/***/ }),

/***/ 7333:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.GetRangeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/getrange
 */
class GetRangeCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["getrange", ...cmd], opts);
    }
}
exports.GetRangeCommand = GetRangeCommand;


/***/ }),

/***/ 9974:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.GetSetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/getset
 */
class GetSetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["getset", ...cmd], opts);
    }
}
exports.GetSetCommand = GetSetCommand;


/***/ }),

/***/ 6490:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HDelCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hdel
 */
class HDelCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hdel", ...cmd], opts);
    }
}
exports.HDelCommand = HDelCommand;


/***/ }),

/***/ 172:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HExistsCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hexists
 */
class HExistsCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hexists", ...cmd], opts);
    }
}
exports.HExistsCommand = HExistsCommand;


/***/ }),

/***/ 5371:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HGetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hget
 */
class HGetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hget", ...cmd], opts);
    }
}
exports.HGetCommand = HGetCommand;


/***/ }),

/***/ 6528:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HGetAllCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
function deserialize(result) {
    if (result.length === 0) {
        return null;
    }
    const obj = {};
    while (result.length >= 2) {
        const key = result.shift();
        const value = result.shift();
        try {
            obj[key] = JSON.parse(value);
        }
        catch {
            obj[key] = value;
        }
    }
    return obj;
}
/**
 * @see https://redis.io/commands/hgetall
 */
class HGetAllCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hgetall", ...cmd], {
            deserialize: (result) => deserialize(result),
            ...opts,
        });
    }
}
exports.HGetAllCommand = HGetAllCommand;


/***/ }),

/***/ 6024:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HIncrByCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hincrby
 */
class HIncrByCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hincrby", ...cmd], opts);
    }
}
exports.HIncrByCommand = HIncrByCommand;


/***/ }),

/***/ 5638:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HIncrByFloatCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hincrbyfloat
 */
class HIncrByFloatCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hincrbyfloat", ...cmd], opts);
    }
}
exports.HIncrByFloatCommand = HIncrByFloatCommand;


/***/ }),

/***/ 4177:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HKeysCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hkeys
 */
class HKeysCommand extends command_js_1.Command {
    constructor([key], opts) {
        super(["hkeys", key], opts);
    }
}
exports.HKeysCommand = HKeysCommand;


/***/ }),

/***/ 8301:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HLenCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hlen
 */
class HLenCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hlen", ...cmd], opts);
    }
}
exports.HLenCommand = HLenCommand;


/***/ }),

/***/ 455:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HMGetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
function deserialize(fields, result) {
    if (result.length === 0 || result.every((field) => field === null)) {
        return null;
    }
    const obj = {};
    for (let i = 0; i < fields.length; i++) {
        try {
            obj[fields[i]] = JSON.parse(result[i]);
        }
        catch {
            obj[fields[i]] = result[i];
        }
    }
    return obj;
}
/**
 * hmget returns an object of all requested fields from a hash
 * The field values are returned as an object like this:
 * ```ts
 * {[fieldName: string]: T | null}
 * ```
 *
 * In case the hash does not exist or all fields are empty `null` is returned
 *
 * @see https://redis.io/commands/hmget
 */
class HMGetCommand extends command_js_1.Command {
    constructor([key, ...fields], opts) {
        super(["hmget", key, ...fields], {
            deserialize: (result) => deserialize(fields, result),
            ...opts,
        });
    }
}
exports.HMGetCommand = HMGetCommand;


/***/ }),

/***/ 5701:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HMSetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hmset
 */
class HMSetCommand extends command_js_1.Command {
    constructor([key, kv], opts) {
        super([
            "hmset",
            key,
            ...Object.entries(kv).flatMap(([field, value]) => [field, value]),
        ], opts);
    }
}
exports.HMSetCommand = HMSetCommand;


/***/ }),

/***/ 913:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HRandFieldCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
function deserialize(result) {
    if (result.length === 0) {
        return null;
    }
    const obj = {};
    while (result.length >= 2) {
        const key = result.shift();
        const value = result.shift();
        try {
            obj[key] = JSON.parse(value);
        }
        catch {
            obj[key] = value;
        }
    }
    return obj;
}
/**
 * @see https://redis.io/commands/hrandfield
 */
class HRandFieldCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        const command = ["hrandfield", cmd[0]];
        if (typeof cmd[1] === "number") {
            command.push(cmd[1]);
        }
        if (cmd[2]) {
            command.push("WITHVALUES");
        }
        super(command, {
            // @ts-ignore TODO:
            deserialize: cmd[2]
                ? (result) => deserialize(result)
                : opts?.deserialize,
            ...opts,
        });
    }
}
exports.HRandFieldCommand = HRandFieldCommand;


/***/ }),

/***/ 9914:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HScanCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hscan
 */
class HScanCommand extends command_js_1.Command {
    constructor([key, cursor, cmdOpts], opts) {
        const command = ["hscan", key, cursor];
        if (cmdOpts?.match) {
            command.push("match", cmdOpts.match);
        }
        if (typeof cmdOpts?.count === "number") {
            command.push("count", cmdOpts.count);
        }
        super(command, opts);
    }
}
exports.HScanCommand = HScanCommand;


/***/ }),

/***/ 3316:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HSetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hset
 */
class HSetCommand extends command_js_1.Command {
    constructor([key, kv], opts) {
        super([
            "hset",
            key,
            ...Object.entries(kv).flatMap(([field, value]) => [field, value]),
        ], opts);
    }
}
exports.HSetCommand = HSetCommand;


/***/ }),

/***/ 7290:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HSetNXCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hsetnx
 */
class HSetNXCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hsetnx", ...cmd], opts);
    }
}
exports.HSetNXCommand = HSetNXCommand;


/***/ }),

/***/ 2203:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HStrLenCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hstrlen
 */
class HStrLenCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hstrlen", ...cmd], opts);
    }
}
exports.HStrLenCommand = HStrLenCommand;


/***/ }),

/***/ 5608:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HValsCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/hvals
 */
class HValsCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["hvals", ...cmd], opts);
    }
}
exports.HValsCommand = HValsCommand;


/***/ }),

/***/ 7193:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.IncrCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/incr
 */
class IncrCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["incr", ...cmd], opts);
    }
}
exports.IncrCommand = IncrCommand;


/***/ }),

/***/ 9336:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.IncrByCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/incrby
 */
class IncrByCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["incrby", ...cmd], opts);
    }
}
exports.IncrByCommand = IncrByCommand;


/***/ }),

/***/ 1687:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.IncrByFloatCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/incrbyfloat
 */
class IncrByFloatCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["incrbyfloat", ...cmd], opts);
    }
}
exports.IncrByFloatCommand = IncrByFloatCommand;


/***/ }),

/***/ 7106:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonArrAppendCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.arrappend
 */
class JsonArrAppendCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.ARRAPPEND", ...cmd], opts);
    }
}
exports.JsonArrAppendCommand = JsonArrAppendCommand;


/***/ }),

/***/ 8057:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonArrIndexCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.arrindex
 */
class JsonArrIndexCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.ARRINDEX", ...cmd], opts);
    }
}
exports.JsonArrIndexCommand = JsonArrIndexCommand;


/***/ }),

/***/ 1280:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonArrInsertCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.arrinsert
 */
class JsonArrInsertCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.ARRINSERT", ...cmd], opts);
    }
}
exports.JsonArrInsertCommand = JsonArrInsertCommand;


/***/ }),

/***/ 6019:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonArrLenCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.arrlen
 */
class JsonArrLenCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.ARRLEN", cmd[0], cmd[1] ?? "$"], opts);
    }
}
exports.JsonArrLenCommand = JsonArrLenCommand;


/***/ }),

/***/ 5259:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonArrPopCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.arrpop
 */
class JsonArrPopCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.ARRPOP", ...cmd], opts);
    }
}
exports.JsonArrPopCommand = JsonArrPopCommand;


/***/ }),

/***/ 1053:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonArrTrimCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.arrtrim
 */
class JsonArrTrimCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        const path = cmd[1] ?? "$";
        const start = cmd[2] ?? 0;
        const stop = cmd[3] ?? 0;
        super(["JSON.ARRTRIM", cmd[0], path, start, stop], opts);
    }
}
exports.JsonArrTrimCommand = JsonArrTrimCommand;


/***/ }),

/***/ 5016:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonClearCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.clear
 */
class JsonClearCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.CLEAR", ...cmd], opts);
    }
}
exports.JsonClearCommand = JsonClearCommand;


/***/ }),

/***/ 4259:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonDelCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.del
 */
class JsonDelCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.DEL", ...cmd], opts);
    }
}
exports.JsonDelCommand = JsonDelCommand;


/***/ }),

/***/ 3946:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonForgetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.forget
 */
class JsonForgetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.FORGET", ...cmd], opts);
    }
}
exports.JsonForgetCommand = JsonForgetCommand;


/***/ }),

/***/ 5203:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonGetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.get
 */
class JsonGetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        const command = ["JSON.GET"];
        if (typeof cmd[1] === "string") {
            // @ts-ignore - we know this is a string
            command.push(...cmd);
        }
        else {
            command.push(cmd[0]);
            if (cmd[1]) {
                if (cmd[1].indent) {
                    command.push("INDENT", cmd[1].indent);
                }
                if (cmd[1].newline) {
                    command.push("NEWLINE", cmd[1].newline);
                }
                if (cmd[1].space) {
                    command.push("SPACE", cmd[1].space);
                }
            }
            // @ts-ignore - we know this is a string
            command.push(...cmd.slice(2));
        }
        super(command, opts);
    }
}
exports.JsonGetCommand = JsonGetCommand;


/***/ }),

/***/ 2977:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonMGetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.mget
 */
class JsonMGetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.MGET", ...cmd[0], cmd[1]], opts);
    }
}
exports.JsonMGetCommand = JsonMGetCommand;


/***/ }),

/***/ 2299:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonNumIncrByCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.numincrby
 */
class JsonNumIncrByCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.NUMINCRBY", ...cmd], opts);
    }
}
exports.JsonNumIncrByCommand = JsonNumIncrByCommand;


/***/ }),

/***/ 4231:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonNumMultByCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.nummultby
 */
class JsonNumMultByCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.NUMMULTBY", ...cmd], opts);
    }
}
exports.JsonNumMultByCommand = JsonNumMultByCommand;


/***/ }),

/***/ 8354:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonObjKeysCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.objkeys
 */
class JsonObjKeysCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.OBJKEYS", ...cmd], opts);
    }
}
exports.JsonObjKeysCommand = JsonObjKeysCommand;


/***/ }),

/***/ 8844:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonObjLenCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.objlen
 */
class JsonObjLenCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.OBJLEN", ...cmd], opts);
    }
}
exports.JsonObjLenCommand = JsonObjLenCommand;


/***/ }),

/***/ 4481:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonRespCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.resp
 */
class JsonRespCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.RESP", ...cmd], opts);
    }
}
exports.JsonRespCommand = JsonRespCommand;


/***/ }),

/***/ 9307:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonSetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.set
 */
class JsonSetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        const command = ["JSON.SET", cmd[0], cmd[1], cmd[2]];
        if (cmd[3]) {
            if (cmd[3].nx) {
                command.push("NX");
            }
            else if (cmd[3].xx) {
                command.push("XX");
            }
        }
        super(command, opts);
    }
}
exports.JsonSetCommand = JsonSetCommand;


/***/ }),

/***/ 259:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonStrAppendCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.strappend
 */
class JsonStrAppendCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.STRAPPEND", ...cmd], opts);
    }
}
exports.JsonStrAppendCommand = JsonStrAppendCommand;


/***/ }),

/***/ 7081:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonStrLenCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.strlen
 */
class JsonStrLenCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.STRLEN", ...cmd], opts);
    }
}
exports.JsonStrLenCommand = JsonStrLenCommand;


/***/ }),

/***/ 1004:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonToggleCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.toggle
 */
class JsonToggleCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.TOGGLE", ...cmd], opts);
    }
}
exports.JsonToggleCommand = JsonToggleCommand;


/***/ }),

/***/ 1431:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.JsonTypeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/json.type
 */
class JsonTypeCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["JSON.TYPE", ...cmd], opts);
    }
}
exports.JsonTypeCommand = JsonTypeCommand;


/***/ }),

/***/ 9407:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.KeysCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/keys
 */
class KeysCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["keys", ...cmd], opts);
    }
}
exports.KeysCommand = KeysCommand;


/***/ }),

/***/ 5531:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LIndexCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
class LIndexCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lindex", ...cmd], opts);
    }
}
exports.LIndexCommand = LIndexCommand;


/***/ }),

/***/ 8812:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LInsertCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
class LInsertCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["linsert", ...cmd], opts);
    }
}
exports.LInsertCommand = LInsertCommand;


/***/ }),

/***/ 2303:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LLenCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/llen
 */
class LLenCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["llen", ...cmd], opts);
    }
}
exports.LLenCommand = LLenCommand;


/***/ }),

/***/ 7147:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LMoveCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/lmove
 */
class LMoveCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lmove", ...cmd], opts);
    }
}
exports.LMoveCommand = LMoveCommand;


/***/ }),

/***/ 7153:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LPopCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/lpop
 */
class LPopCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lpop", ...cmd], opts);
    }
}
exports.LPopCommand = LPopCommand;


/***/ }),

/***/ 3003:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LPosCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/lpos
 */
class LPosCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        const args = ["lpos", cmd[0], cmd[1]];
        if (typeof cmd[2]?.rank === "number") {
            args.push("rank", cmd[2].rank);
        }
        if (typeof cmd[2]?.count === "number") {
            args.push("count", cmd[2].count);
        }
        if (typeof cmd[2]?.maxLen === "number") {
            args.push("maxLen", cmd[2].maxLen);
        }
        super(args, opts);
    }
}
exports.LPosCommand = LPosCommand;


/***/ }),

/***/ 521:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LPushCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/lpush
 */
class LPushCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lpush", ...cmd], opts);
    }
}
exports.LPushCommand = LPushCommand;


/***/ }),

/***/ 782:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LPushXCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/lpushx
 */
class LPushXCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lpushx", ...cmd], opts);
    }
}
exports.LPushXCommand = LPushXCommand;


/***/ }),

/***/ 7580:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LRangeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
class LRangeCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lrange", ...cmd], opts);
    }
}
exports.LRangeCommand = LRangeCommand;


/***/ }),

/***/ 4366:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LRemCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
class LRemCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lrem", ...cmd], opts);
    }
}
exports.LRemCommand = LRemCommand;


/***/ }),

/***/ 5207:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LSetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
class LSetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["lset", ...cmd], opts);
    }
}
exports.LSetCommand = LSetCommand;


/***/ }),

/***/ 3066:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.LTrimCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
class LTrimCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["ltrim", ...cmd], opts);
    }
}
exports.LTrimCommand = LTrimCommand;


/***/ }),

/***/ 7614:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.MGetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/mget
 */
class MGetCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["mget", ...cmd], opts);
    }
}
exports.MGetCommand = MGetCommand;


/***/ }),

/***/ 9899:
/***/ (function(__unused_webpack_module, exports, __nccwpck_require__) {

"use strict";

var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __exportStar = (this && this.__exportStar) || function(m, exports) {
    for (var p in m) if (p !== "default" && !Object.prototype.hasOwnProperty.call(exports, p)) __createBinding(exports, m, p);
};
Object.defineProperty(exports, "__esModule", ({ value: true }));
__exportStar(__nccwpck_require__(8835), exports);
__exportStar(__nccwpck_require__(406), exports);
__exportStar(__nccwpck_require__(3539), exports);
__exportStar(__nccwpck_require__(4045), exports);
__exportStar(__nccwpck_require__(740), exports);
__exportStar(__nccwpck_require__(2018), exports);
__exportStar(__nccwpck_require__(12), exports);
__exportStar(__nccwpck_require__(8429), exports);
__exportStar(__nccwpck_require__(7142), exports);
__exportStar(__nccwpck_require__(8920), exports);
__exportStar(__nccwpck_require__(6263), exports);
__exportStar(__nccwpck_require__(9800), exports);
__exportStar(__nccwpck_require__(2333), exports);
__exportStar(__nccwpck_require__(5611), exports);
__exportStar(__nccwpck_require__(6139), exports);
__exportStar(__nccwpck_require__(4821), exports);
__exportStar(__nccwpck_require__(5896), exports);
__exportStar(__nccwpck_require__(3802), exports);
__exportStar(__nccwpck_require__(7307), exports);
__exportStar(__nccwpck_require__(2296), exports);
__exportStar(__nccwpck_require__(7333), exports);
__exportStar(__nccwpck_require__(9974), exports);
__exportStar(__nccwpck_require__(6490), exports);
__exportStar(__nccwpck_require__(172), exports);
__exportStar(__nccwpck_require__(5371), exports);
__exportStar(__nccwpck_require__(6528), exports);
__exportStar(__nccwpck_require__(6024), exports);
__exportStar(__nccwpck_require__(5638), exports);
__exportStar(__nccwpck_require__(4177), exports);
__exportStar(__nccwpck_require__(8301), exports);
__exportStar(__nccwpck_require__(455), exports);
__exportStar(__nccwpck_require__(5701), exports);
__exportStar(__nccwpck_require__(913), exports);
__exportStar(__nccwpck_require__(9914), exports);
__exportStar(__nccwpck_require__(3316), exports);
__exportStar(__nccwpck_require__(7290), exports);
__exportStar(__nccwpck_require__(2203), exports);
__exportStar(__nccwpck_require__(5608), exports);
__exportStar(__nccwpck_require__(7193), exports);
__exportStar(__nccwpck_require__(9336), exports);
__exportStar(__nccwpck_require__(1687), exports);
__exportStar(__nccwpck_require__(7106), exports);
__exportStar(__nccwpck_require__(8057), exports);
__exportStar(__nccwpck_require__(1280), exports);
__exportStar(__nccwpck_require__(6019), exports);
__exportStar(__nccwpck_require__(5259), exports);
__exportStar(__nccwpck_require__(1053), exports);
__exportStar(__nccwpck_require__(5016), exports);
__exportStar(__nccwpck_require__(4259), exports);
__exportStar(__nccwpck_require__(3946), exports);
__exportStar(__nccwpck_require__(5203), exports);
__exportStar(__nccwpck_require__(2977), exports);
__exportStar(__nccwpck_require__(2299), exports);
__exportStar(__nccwpck_require__(4231), exports);
__exportStar(__nccwpck_require__(8354), exports);
__exportStar(__nccwpck_require__(8844), exports);
__exportStar(__nccwpck_require__(4481), exports);
__exportStar(__nccwpck_require__(9307), exports);
__exportStar(__nccwpck_require__(259), exports);
__exportStar(__nccwpck_require__(7081), exports);
__exportStar(__nccwpck_require__(1004), exports);
__exportStar(__nccwpck_require__(1431), exports);
__exportStar(__nccwpck_require__(9407), exports);
__exportStar(__nccwpck_require__(5531), exports);
__exportStar(__nccwpck_require__(8812), exports);
__exportStar(__nccwpck_require__(2303), exports);
__exportStar(__nccwpck_require__(7147), exports);
__exportStar(__nccwpck_require__(7153), exports);
__exportStar(__nccwpck_require__(3003), exports);
__exportStar(__nccwpck_require__(521), exports);
__exportStar(__nccwpck_require__(782), exports);
__exportStar(__nccwpck_require__(7580), exports);
__exportStar(__nccwpck_require__(4366), exports);
__exportStar(__nccwpck_require__(5207), exports);
__exportStar(__nccwpck_require__(3066), exports);
__exportStar(__nccwpck_require__(7614), exports);
__exportStar(__nccwpck_require__(1425), exports);
__exportStar(__nccwpck_require__(9099), exports);
__exportStar(__nccwpck_require__(8766), exports);
__exportStar(__nccwpck_require__(4780), exports);
__exportStar(__nccwpck_require__(8329), exports);
__exportStar(__nccwpck_require__(9665), exports);
__exportStar(__nccwpck_require__(7963), exports);
__exportStar(__nccwpck_require__(1865), exports);
__exportStar(__nccwpck_require__(6537), exports);
__exportStar(__nccwpck_require__(502), exports);
__exportStar(__nccwpck_require__(3269), exports);
__exportStar(__nccwpck_require__(8231), exports);
__exportStar(__nccwpck_require__(6201), exports);
__exportStar(__nccwpck_require__(4262), exports);
__exportStar(__nccwpck_require__(3631), exports);
__exportStar(__nccwpck_require__(6017), exports);
__exportStar(__nccwpck_require__(2869), exports);
__exportStar(__nccwpck_require__(308), exports);
__exportStar(__nccwpck_require__(1092), exports);
__exportStar(__nccwpck_require__(9746), exports);
__exportStar(__nccwpck_require__(5123), exports);
__exportStar(__nccwpck_require__(5202), exports);
__exportStar(__nccwpck_require__(5621), exports);
__exportStar(__nccwpck_require__(4537), exports);
__exportStar(__nccwpck_require__(3585), exports);
__exportStar(__nccwpck_require__(5407), exports);
__exportStar(__nccwpck_require__(8106), exports);
__exportStar(__nccwpck_require__(7496), exports);
__exportStar(__nccwpck_require__(8988), exports);
__exportStar(__nccwpck_require__(7547), exports);
__exportStar(__nccwpck_require__(8725), exports);
__exportStar(__nccwpck_require__(6757), exports);
__exportStar(__nccwpck_require__(6063), exports);
__exportStar(__nccwpck_require__(1663), exports);
__exportStar(__nccwpck_require__(5888), exports);
__exportStar(__nccwpck_require__(3370), exports);
__exportStar(__nccwpck_require__(1985), exports);
__exportStar(__nccwpck_require__(6209), exports);
__exportStar(__nccwpck_require__(919), exports);
__exportStar(__nccwpck_require__(3000), exports);
__exportStar(__nccwpck_require__(7491), exports);
__exportStar(__nccwpck_require__(1252), exports);
__exportStar(__nccwpck_require__(9395), exports);
__exportStar(__nccwpck_require__(9237), exports);
__exportStar(__nccwpck_require__(2199), exports);
__exportStar(__nccwpck_require__(6639), exports);
__exportStar(__nccwpck_require__(5412), exports);
__exportStar(__nccwpck_require__(7847), exports);
__exportStar(__nccwpck_require__(1583), exports);
__exportStar(__nccwpck_require__(2856), exports);
__exportStar(__nccwpck_require__(3543), exports);
__exportStar(__nccwpck_require__(9486), exports);
__exportStar(__nccwpck_require__(190), exports);
__exportStar(__nccwpck_require__(7066), exports);
__exportStar(__nccwpck_require__(6631), exports);
__exportStar(__nccwpck_require__(6112), exports);
__exportStar(__nccwpck_require__(5859), exports);
__exportStar(__nccwpck_require__(8798), exports);
__exportStar(__nccwpck_require__(9253), exports);
__exportStar(__nccwpck_require__(2696), exports);
__exportStar(__nccwpck_require__(5675), exports);
__exportStar(__nccwpck_require__(5402), exports);
__exportStar(__nccwpck_require__(5717), exports);
__exportStar(__nccwpck_require__(3603), exports);


/***/ }),

/***/ 1425:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.MSetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/mset
 */
class MSetCommand extends command_js_1.Command {
    constructor([kv], opts) {
        super([
            "mset",
            ...Object.entries(kv).flatMap(([key, value]) => [key, value]),
        ], opts);
    }
}
exports.MSetCommand = MSetCommand;


/***/ }),

/***/ 9099:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.MSetNXCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/msetnx
 */
class MSetNXCommand extends command_js_1.Command {
    constructor([kv], opts) {
        super(["msetnx", ...Object.entries(kv).flatMap((_) => _)], opts);
    }
}
exports.MSetNXCommand = MSetNXCommand;


/***/ }),

/***/ 8766:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.PersistCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/persist
 */
class PersistCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["persist", ...cmd], opts);
    }
}
exports.PersistCommand = PersistCommand;


/***/ }),

/***/ 4780:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.PExpireCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/pexpire
 */
class PExpireCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["pexpire", ...cmd], opts);
    }
}
exports.PExpireCommand = PExpireCommand;


/***/ }),

/***/ 8329:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.PExpireAtCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/pexpireat
 */
class PExpireAtCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["pexpireat", ...cmd], opts);
    }
}
exports.PExpireAtCommand = PExpireAtCommand;


/***/ }),

/***/ 9665:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.PingCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/ping
 */
class PingCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        const command = ["ping"];
        if (typeof cmd !== "undefined" && typeof cmd[0] !== "undefined") {
            command.push(cmd[0]);
        }
        super(command, opts);
    }
}
exports.PingCommand = PingCommand;


/***/ }),

/***/ 7963:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.PSetEXCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/psetex
 */
class PSetEXCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["psetex", ...cmd], opts);
    }
}
exports.PSetEXCommand = PSetEXCommand;


/***/ }),

/***/ 1865:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.PTtlCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/pttl
 */
class PTtlCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["pttl", ...cmd], opts);
    }
}
exports.PTtlCommand = PTtlCommand;


/***/ }),

/***/ 6537:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.PublishCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/publish
 */
class PublishCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["publish", ...cmd], opts);
    }
}
exports.PublishCommand = PublishCommand;


/***/ }),

/***/ 502:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.RandomKeyCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/randomkey
 */
class RandomKeyCommand extends command_js_1.Command {
    constructor(opts) {
        super(["randomkey"], opts);
    }
}
exports.RandomKeyCommand = RandomKeyCommand;


/***/ }),

/***/ 3269:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.RenameCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/rename
 */
class RenameCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["rename", ...cmd], opts);
    }
}
exports.RenameCommand = RenameCommand;


/***/ }),

/***/ 8231:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.RenameNXCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/renamenx
 */
class RenameNXCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["renamenx", ...cmd], opts);
    }
}
exports.RenameNXCommand = RenameNXCommand;


/***/ }),

/***/ 6201:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.RPopCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/rpop
 */
class RPopCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["rpop", ...cmd], opts);
    }
}
exports.RPopCommand = RPopCommand;


/***/ }),

/***/ 4262:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.RPushCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/rpush
 */
class RPushCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["rpush", ...cmd], opts);
    }
}
exports.RPushCommand = RPushCommand;


/***/ }),

/***/ 3631:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.RPushXCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/rpushx
 */
class RPushXCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["rpushx", ...cmd], opts);
    }
}
exports.RPushXCommand = RPushXCommand;


/***/ }),

/***/ 6017:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SAddCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sadd
 */
class SAddCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sadd", ...cmd], opts);
    }
}
exports.SAddCommand = SAddCommand;


/***/ }),

/***/ 2869:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ScanCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/scan
 */
class ScanCommand extends command_js_1.Command {
    constructor([cursor, opts], cmdOpts) {
        const command = ["scan", cursor];
        if (opts?.match) {
            command.push("match", opts.match);
        }
        if (typeof opts?.count === "number") {
            command.push("count", opts.count);
        }
        if (opts?.type && opts.type.length > 0) {
            command.push("type", opts.type);
        }
        super(command, cmdOpts);
    }
}
exports.ScanCommand = ScanCommand;


/***/ }),

/***/ 308:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SCardCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/scard
 */
class SCardCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["scard", ...cmd], opts);
    }
}
exports.SCardCommand = SCardCommand;


/***/ }),

/***/ 1092:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ScriptExistsCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/script-exists
 */
class ScriptExistsCommand extends command_js_1.Command {
    constructor(hashes, opts) {
        super(["script", "exists", ...hashes], {
            deserialize: (result) => result,
            ...opts,
        });
    }
}
exports.ScriptExistsCommand = ScriptExistsCommand;


/***/ }),

/***/ 9746:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ScriptFlushCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/script-flush
 */
class ScriptFlushCommand extends command_js_1.Command {
    constructor([opts], cmdOpts) {
        const cmd = ["script", "flush"];
        if (opts?.sync) {
            cmd.push("sync");
        }
        else if (opts?.async) {
            cmd.push("async");
        }
        super(cmd, cmdOpts);
    }
}
exports.ScriptFlushCommand = ScriptFlushCommand;


/***/ }),

/***/ 5123:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ScriptLoadCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/script-load
 */
class ScriptLoadCommand extends command_js_1.Command {
    constructor(args, opts) {
        super(["script", "load", ...args], opts);
    }
}
exports.ScriptLoadCommand = ScriptLoadCommand;


/***/ }),

/***/ 5202:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SDiffCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sdiff
 */
class SDiffCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sdiff", ...cmd], opts);
    }
}
exports.SDiffCommand = SDiffCommand;


/***/ }),

/***/ 5621:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SDiffStoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sdiffstore
 */
class SDiffStoreCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sdiffstore", ...cmd], opts);
    }
}
exports.SDiffStoreCommand = SDiffStoreCommand;


/***/ }),

/***/ 4537:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SetCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/set
 */
class SetCommand extends command_js_1.Command {
    constructor([key, value, opts], cmdOpts) {
        const command = ["set", key, value];
        if (opts) {
            if ("nx" in opts && opts.nx) {
                command.push("nx");
            }
            else if ("xx" in opts && opts.xx) {
                command.push("xx");
            }
            if ("get" in opts && opts.get) {
                command.push("get");
            }
            if ("ex" in opts && typeof opts.ex === "number") {
                command.push("ex", opts.ex);
            }
            else if ("px" in opts && typeof opts.px === "number") {
                command.push("px", opts.px);
            }
            else if ("exat" in opts && typeof opts.exat === "number") {
                command.push("exat", opts.exat);
            }
            else if ("pxat" in opts && typeof opts.pxat === "number") {
                command.push("pxat", opts.pxat);
            }
            else if ("keepTtl" in opts && opts.keepTtl) {
                command.push("keepTtl", opts.keepTtl);
            }
        }
        super(command, cmdOpts);
    }
}
exports.SetCommand = SetCommand;


/***/ }),

/***/ 3585:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SetBitCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/setbit
 */
class SetBitCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["setbit", ...cmd], opts);
    }
}
exports.SetBitCommand = SetBitCommand;


/***/ }),

/***/ 5407:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SetExCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/setex
 */
class SetExCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["setex", ...cmd], opts);
    }
}
exports.SetExCommand = SetExCommand;


/***/ }),

/***/ 8106:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SetNxCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/setnx
 */
class SetNxCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["setnx", ...cmd], opts);
    }
}
exports.SetNxCommand = SetNxCommand;


/***/ }),

/***/ 7496:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SetRangeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/setrange
 */
class SetRangeCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["setrange", ...cmd], opts);
    }
}
exports.SetRangeCommand = SetRangeCommand;


/***/ }),

/***/ 8988:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SInterCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sinter
 */
class SInterCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sinter", ...cmd], opts);
    }
}
exports.SInterCommand = SInterCommand;


/***/ }),

/***/ 7547:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SInterStoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sinterstore
 */
class SInterStoreCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sinterstore", ...cmd], opts);
    }
}
exports.SInterStoreCommand = SInterStoreCommand;


/***/ }),

/***/ 8725:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SIsMemberCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sismember
 */
class SIsMemberCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sismember", ...cmd], opts);
    }
}
exports.SIsMemberCommand = SIsMemberCommand;


/***/ }),

/***/ 6757:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SMembersCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/smembers
 */
class SMembersCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["smembers", ...cmd], opts);
    }
}
exports.SMembersCommand = SMembersCommand;


/***/ }),

/***/ 6063:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SMIsMemberCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/smismember
 */
class SMIsMemberCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["smismember", cmd[0], ...cmd[1]], opts);
    }
}
exports.SMIsMemberCommand = SMIsMemberCommand;


/***/ }),

/***/ 1663:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SMoveCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/smove
 */
class SMoveCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["smove", ...cmd], opts);
    }
}
exports.SMoveCommand = SMoveCommand;


/***/ }),

/***/ 5888:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SPopCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/spop
 */
class SPopCommand extends command_js_1.Command {
    constructor([key, count], opts) {
        const command = ["spop", key];
        if (typeof count === "number") {
            command.push(count);
        }
        super(command, opts);
    }
}
exports.SPopCommand = SPopCommand;


/***/ }),

/***/ 3370:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SRandMemberCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/srandmember
 */
class SRandMemberCommand extends command_js_1.Command {
    constructor([key, count], opts) {
        const command = ["srandmember", key];
        if (typeof count === "number") {
            command.push(count);
        }
        super(command, opts);
    }
}
exports.SRandMemberCommand = SRandMemberCommand;


/***/ }),

/***/ 1985:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SRemCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/srem
 */
class SRemCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["srem", ...cmd], opts);
    }
}
exports.SRemCommand = SRemCommand;


/***/ }),

/***/ 6209:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SScanCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sscan
 */
class SScanCommand extends command_js_1.Command {
    constructor([key, cursor, opts], cmdOpts) {
        const command = ["sscan", key, cursor];
        if (opts?.match) {
            command.push("match", opts.match);
        }
        if (typeof opts?.count === "number") {
            command.push("count", opts.count);
        }
        super(command, cmdOpts);
    }
}
exports.SScanCommand = SScanCommand;


/***/ }),

/***/ 919:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.StrLenCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/strlen
 */
class StrLenCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["strlen", ...cmd], opts);
    }
}
exports.StrLenCommand = StrLenCommand;


/***/ }),

/***/ 3000:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SUnionCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sunion
 */
class SUnionCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sunion", ...cmd], opts);
    }
}
exports.SUnionCommand = SUnionCommand;


/***/ }),

/***/ 7491:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.SUnionStoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/sunionstore
 */
class SUnionStoreCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["sunionstore", ...cmd], opts);
    }
}
exports.SUnionStoreCommand = SUnionStoreCommand;


/***/ }),

/***/ 1252:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.TimeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/time
 */
class TimeCommand extends command_js_1.Command {
    constructor(opts) {
        super(["time"], opts);
    }
}
exports.TimeCommand = TimeCommand;


/***/ }),

/***/ 9395:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.TouchCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/touch
 */
class TouchCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["touch", ...cmd], opts);
    }
}
exports.TouchCommand = TouchCommand;


/***/ }),

/***/ 9237:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.TtlCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/ttl
 */
class TtlCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["ttl", ...cmd], opts);
    }
}
exports.TtlCommand = TtlCommand;


/***/ }),

/***/ 2199:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.TypeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/type
 */
class TypeCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["type", ...cmd], opts);
    }
}
exports.TypeCommand = TypeCommand;


/***/ }),

/***/ 6639:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.UnlinkCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/unlink
 */
class UnlinkCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["unlink", ...cmd], opts);
    }
}
exports.UnlinkCommand = UnlinkCommand;


/***/ }),

/***/ 5412:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZAddCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zadd
 */
class ZAddCommand extends command_js_1.Command {
    constructor([key, arg1, ...arg2], opts) {
        const command = ["zadd", key];
        if ("nx" in arg1 && arg1.nx) {
            command.push("nx");
        }
        else if ("xx" in arg1 && arg1.xx) {
            command.push("xx");
        }
        if ("ch" in arg1 && arg1.ch) {
            command.push("ch");
        }
        if ("incr" in arg1 && arg1.incr) {
            command.push("incr");
        }
        if ("score" in arg1 && "member" in arg1) {
            command.push(arg1.score, arg1.member);
        }
        command.push(...arg2.flatMap(({ score, member }) => [score, member]));
        super(command, opts);
    }
}
exports.ZAddCommand = ZAddCommand;


/***/ }),

/***/ 7847:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZCardCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zcard
 */
class ZCardCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zcard", ...cmd], opts);
    }
}
exports.ZCardCommand = ZCardCommand;


/***/ }),

/***/ 1583:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZCountCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zcount
 */
class ZCountCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zcount", ...cmd], opts);
    }
}
exports.ZCountCommand = ZCountCommand;


/***/ }),

/***/ 760:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZDiffStoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zdiffstore
 */
class ZDiffStoreCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zdiffstore", ...cmd], opts);
    }
}
exports.ZDiffStoreCommand = ZDiffStoreCommand;


/***/ }),

/***/ 2856:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZIncrByCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zincrby
 */
class ZIncrByCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zincrby", ...cmd], opts);
    }
}
exports.ZIncrByCommand = ZIncrByCommand;


/***/ }),

/***/ 3543:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZInterStoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zInterstore
 */
class ZInterStoreCommand extends command_js_1.Command {
    constructor([destination, numKeys, keyOrKeys, opts], cmdOpts) {
        const command = ["zinterstore", destination, numKeys];
        if (Array.isArray(keyOrKeys)) {
            command.push(...keyOrKeys);
        }
        else {
            command.push(keyOrKeys);
        }
        if (opts) {
            if ("weights" in opts && opts.weights) {
                command.push("weights", ...opts.weights);
            }
            else if ("weight" in opts && typeof opts.weight === "number") {
                command.push("weights", opts.weight);
            }
            if ("aggregate" in opts) {
                command.push("aggregate", opts.aggregate);
            }
        }
        super(command, cmdOpts);
    }
}
exports.ZInterStoreCommand = ZInterStoreCommand;


/***/ }),

/***/ 9486:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZLexCountCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zlexcount
 */
class ZLexCountCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zlexcount", ...cmd], opts);
    }
}
exports.ZLexCountCommand = ZLexCountCommand;


/***/ }),

/***/ 6475:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZMScoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zmscore
 */
class ZMScoreCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        const [key, members] = cmd;
        super(["zmscore", key, ...members], opts);
    }
}
exports.ZMScoreCommand = ZMScoreCommand;


/***/ }),

/***/ 190:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZPopMaxCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zpopmax
 */
class ZPopMaxCommand extends command_js_1.Command {
    constructor([key, count], opts) {
        const command = ["zpopmax", key];
        if (typeof count === "number") {
            command.push(count);
        }
        super(command, opts);
    }
}
exports.ZPopMaxCommand = ZPopMaxCommand;


/***/ }),

/***/ 7066:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZPopMinCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zpopmin
 */
class ZPopMinCommand extends command_js_1.Command {
    constructor([key, count], opts) {
        const command = ["zpopmin", key];
        if (typeof count === "number") {
            command.push(count);
        }
        super(command, opts);
    }
}
exports.ZPopMinCommand = ZPopMinCommand;


/***/ }),

/***/ 6631:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZRangeCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zrange
 */
class ZRangeCommand extends command_js_1.Command {
    constructor([key, min, max, opts], cmdOpts) {
        const command = ["zrange", key, min, max];
        // Either byScore or byLex is allowed
        if (opts?.byScore) {
            command.push("byscore");
        }
        if (opts?.byLex) {
            command.push("bylex");
        }
        if (opts?.rev) {
            command.push("rev");
        }
        if (typeof opts?.count !== "undefined" && typeof opts?.offset !== "undefined") {
            command.push("limit", opts.offset, opts.count);
        }
        if (opts?.withScores) {
            command.push("withscores");
        }
        super(command, cmdOpts);
    }
}
exports.ZRangeCommand = ZRangeCommand;


/***/ }),

/***/ 6112:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZRankCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 *  @see https://redis.io/commands/zrank
 */
class ZRankCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zrank", ...cmd], opts);
    }
}
exports.ZRankCommand = ZRankCommand;


/***/ }),

/***/ 5859:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZRemCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zrem
 */
class ZRemCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zrem", ...cmd], opts);
    }
}
exports.ZRemCommand = ZRemCommand;


/***/ }),

/***/ 8798:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZRemRangeByLexCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zremrangebylex
 */
class ZRemRangeByLexCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zremrangebylex", ...cmd], opts);
    }
}
exports.ZRemRangeByLexCommand = ZRemRangeByLexCommand;


/***/ }),

/***/ 9253:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZRemRangeByRankCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zremrangebyrank
 */
class ZRemRangeByRankCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zremrangebyrank", ...cmd], opts);
    }
}
exports.ZRemRangeByRankCommand = ZRemRangeByRankCommand;


/***/ }),

/***/ 2696:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZRemRangeByScoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zremrangebyscore
 */
class ZRemRangeByScoreCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zremrangebyscore", ...cmd], opts);
    }
}
exports.ZRemRangeByScoreCommand = ZRemRangeByScoreCommand;


/***/ }),

/***/ 5675:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZRevRankCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 *  @see https://redis.io/commands/zrevrank
 */
class ZRevRankCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zrevrank", ...cmd], opts);
    }
}
exports.ZRevRankCommand = ZRevRankCommand;


/***/ }),

/***/ 5402:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZScanCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zscan
 */
class ZScanCommand extends command_js_1.Command {
    constructor([key, cursor, opts], cmdOpts) {
        const command = ["zscan", key, cursor];
        if (opts?.match) {
            command.push("match", opts.match);
        }
        if (typeof opts?.count === "number") {
            command.push("count", opts.count);
        }
        super(command, cmdOpts);
    }
}
exports.ZScanCommand = ZScanCommand;


/***/ }),

/***/ 5717:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZScoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zscore
 */
class ZScoreCommand extends command_js_1.Command {
    constructor(cmd, opts) {
        super(["zscore", ...cmd], opts);
    }
}
exports.ZScoreCommand = ZScoreCommand;


/***/ }),

/***/ 3603:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.ZUnionStoreCommand = void 0;
const command_js_1 = __nccwpck_require__(740);
/**
 * @see https://redis.io/commands/zunionstore
 */
class ZUnionStoreCommand extends command_js_1.Command {
    constructor([destination, numKeys, keyOrKeys, opts], cmdOpts) {
        const command = ["zunionstore", destination, numKeys];
        if (Array.isArray(keyOrKeys)) {
            command.push(...keyOrKeys);
        }
        else {
            command.push(keyOrKeys);
        }
        if (opts) {
            if ("weights" in opts && opts.weights) {
                command.push("weights", ...opts.weights);
            }
            else if ("weight" in opts && typeof opts.weight === "number") {
                command.push("weights", opts.weight);
            }
            if ("aggregate" in opts) {
                command.push("aggregate", opts.aggregate);
            }
        }
        super(command, cmdOpts);
    }
}
exports.ZUnionStoreCommand = ZUnionStoreCommand;


/***/ }),

/***/ 2564:
/***/ ((__unused_webpack_module, exports) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.UpstashError = void 0;
/**
 * Result of a bad request to upstash
 */
class UpstashError extends Error {
    constructor(message) {
        super(message);
        this.name = "UpstashError";
    }
}
exports.UpstashError = UpstashError;


/***/ }),

/***/ 2022:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.HttpClient = void 0;
const error_js_1 = __nccwpck_require__(2564);
class HttpClient {
    constructor(config) {
        Object.defineProperty(this, "baseUrl", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "headers", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "options", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "retry", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        this.options = {
            backend: config.options?.backend,
            agent: config.agent,
            responseEncoding: config.responseEncoding ?? "base64",
            cache: config.cache,
        };
        this.baseUrl = config.baseUrl.replace(/\/$/, "");
        this.headers = {
            "Content-Type": "application/json",
            ...config.headers,
        };
        if (this.options.responseEncoding === "base64") {
            this.headers["Upstash-Encoding"] = "base64";
        }
        if (typeof config?.retry === "boolean" && config?.retry === false) {
            this.retry = {
                attempts: 1,
                backoff: () => 0,
            };
        }
        else {
            this.retry = {
                attempts: config?.retry?.retries ?? 5,
                backoff: config?.retry?.backoff ??
                    ((retryCount) => Math.exp(retryCount) * 50),
            };
        }
    }
    mergeTelemetry(telemetry) {
        function merge(obj, key, value) {
            if (!value) {
                return obj;
            }
            if (obj[key]) {
                obj[key] = [obj[key], value].join(",");
            }
            else {
                obj[key] = value;
            }
            return obj;
        }
        this.headers = merge(this.headers, "Upstash-Telemetry-Runtime", telemetry.runtime);
        this.headers = merge(this.headers, "Upstash-Telemetry-Platform", telemetry.platform);
        this.headers = merge(this.headers, "Upstash-Telemetry-Sdk", telemetry.sdk);
    }
    async request(req) {
        const requestOptions = {
            cache: this.options.cache,
            method: "POST",
            headers: this.headers,
            body: JSON.stringify(req.body),
            keepalive: true,
            agent: this.options?.agent,
            /**
             * Fastly specific
             */
            backend: this.options?.backend,
        };
        let res = null;
        let error = null;
        for (let i = 0; i <= this.retry.attempts; i++) {
            try {
                res = await fetch([this.baseUrl, ...(req.path ?? [])].join("/"), requestOptions);
                break;
            }
            catch (err) {
                error = err;
                await new Promise((r) => setTimeout(r, this.retry.backoff(i)));
            }
        }
        if (!res) {
            throw error ?? new Error("Exhausted all retries");
        }
        const body = (await res.json());
        if (!res.ok) {
            throw new error_js_1.UpstashError(body.error);
        }
        if (this.options?.responseEncoding === "base64") {
            return Array.isArray(body) ? body.map(decode) : decode(body);
        }
        return body;
    }
}
exports.HttpClient = HttpClient;
function base64decode(b64) {
    let dec = "";
    try {
        /**
         * Using only atob() is not enough because it doesn't work with unicode characters
         */
        const binString = atob(b64);
        const size = binString.length;
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) {
            bytes[i] = binString.charCodeAt(i);
        }
        dec = new TextDecoder().decode(bytes);
    }
    catch {
        dec = b64;
    }
    return dec;
    // try {
    //   return decodeURIComponent(dec);
    // } catch {
    //   return dec;
    // }
}
function decode(raw) {
    let result = undefined;
    switch (typeof raw.result) {
        case "undefined":
            return raw;
        case "number": {
            result = raw.result;
            break;
        }
        case "object": {
            if (Array.isArray(raw.result)) {
                result = raw.result.map((v) => typeof v === "string"
                    ? base64decode(v)
                    : Array.isArray(v)
                        ? v.map(base64decode)
                        : v);
            }
            else {
                // If it's not an array it must be null
                // Apparently null is an object in javascript
                result = null;
            }
            break;
        }
        case "string": {
            result = raw.result === "OK" ? "OK" : base64decode(raw.result);
            break;
        }
        default:
            break;
    }
    return { result, error: raw.error };
}


/***/ }),

/***/ 262:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.Pipeline = void 0;
const mod_js_1 = __nccwpck_require__(9899);
const error_js_1 = __nccwpck_require__(2564);
const zmscore_js_1 = __nccwpck_require__(6475);
const hrandfield_js_1 = __nccwpck_require__(913);
const zdiffstore_js_1 = __nccwpck_require__(760);
/**
 * Upstash REST API supports command pipelining to send multiple commands in
 * batch, instead of sending each command one by one and waiting for a response.
 * When using pipelines, several commands are sent using a single HTTP request,
 * and a single JSON array response is returned. Each item in the response array
 * corresponds to the command in the same order within the pipeline.
 *
 * **NOTE:**
 *
 * Execution of the pipeline is not atomic. Even though each command in
 * the pipeline will be executed in order, commands sent by other clients can
 * interleave with the pipeline.
 *
 * **Examples:**
 *
 * ```ts
 *  const p = redis.pipeline() // or redis.multi()
 * p.set("key","value")
 * p.get("key")
 * const res = await p.exec()
 * ```
 *
 * You can also chain commands together
 * ```ts
 * const p = redis.pipeline()
 * const res = await p.set("key","value").get("key").exec()
 * ```
 *
 * Return types are inferred if all commands are chained, but you can still
 * override the response type manually:
 * ```ts
 *  redis.pipeline()
 *   .set("key", { greeting: "hello"})
 *   .get("key")
 *   .exec<["OK", { greeting: string } ]>()
 *
 * ```
 */
class Pipeline {
    constructor(opts) {
        Object.defineProperty(this, "client", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "commands", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "commandOptions", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "multiExec", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        /**
         * Send the pipeline request to upstash.
         *
         * Returns an array with the results of all pipelined commands.
         *
         * If all commands are statically chained from start to finish, types are inferred. You can still define a return type manually if necessary though:
         * ```ts
         * const p = redis.pipeline()
         * p.get("key")
         * const result = p.exec<[{ greeting: string }]>()
         * ```
         */
        Object.defineProperty(this, "exec", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: async () => {
                if (this.commands.length === 0) {
                    throw new Error("Pipeline is empty");
                }
                const path = this.multiExec ? ["multi-exec"] : ["pipeline"];
                const res = (await this.client.request({
                    path,
                    body: Object.values(this.commands).map((c) => c.command),
                }));
                return res.map(({ error, result }, i) => {
                    if (error) {
                        throw new error_js_1.UpstashError(`Command ${i + 1} [ ${this.commands[i].command[0]} ] failed: ${error}`);
                    }
                    return this.commands[i].deserialize(result);
                });
            }
        });
        /**
         * @see https://redis.io/commands/append
         */
        Object.defineProperty(this, "append", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.AppendCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/bitcount
         */
        Object.defineProperty(this, "bitcount", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.BitCountCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/bitop
         */
        Object.defineProperty(this, "bitop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (op, destinationKey, sourceKey, ...sourceKeys) => this.chain(new mod_js_1.BitOpCommand([op, destinationKey, sourceKey, ...sourceKeys], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/bitpos
         */
        Object.defineProperty(this, "bitpos", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.BitPosCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zdiffstore
         */
        Object.defineProperty(this, "zdiffstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new zdiffstore_js_1.ZDiffStoreCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/dbsize
         */
        Object.defineProperty(this, "dbsize", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => this.chain(new mod_js_1.DBSizeCommand(this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/decr
         */
        Object.defineProperty(this, "decr", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.DecrCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/decrby
         */
        Object.defineProperty(this, "decrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.DecrByCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/del
         */
        Object.defineProperty(this, "del", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.DelCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/echo
         */
        Object.defineProperty(this, "echo", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.EchoCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/eval
         */
        Object.defineProperty(this, "eval", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.EvalCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/evalsha
         */
        Object.defineProperty(this, "evalsha", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.EvalshaCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/exists
         */
        Object.defineProperty(this, "exists", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ExistsCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/expire
         */
        Object.defineProperty(this, "expire", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ExpireCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/expireat
         */
        Object.defineProperty(this, "expireat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ExpireAtCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/flushall
         */
        Object.defineProperty(this, "flushall", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (args) => this.chain(new mod_js_1.FlushAllCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/flushdb
         */
        Object.defineProperty(this, "flushdb", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.FlushDBCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/get
         */
        Object.defineProperty(this, "get", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.GetCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/getbit
         */
        Object.defineProperty(this, "getbit", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.GetBitCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/getdel
         */
        Object.defineProperty(this, "getdel", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.GetDelCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/getrange
         */
        Object.defineProperty(this, "getrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.GetRangeCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/getset
         */
        Object.defineProperty(this, "getset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, value) => this.chain(new mod_js_1.GetSetCommand([key, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hdel
         */
        Object.defineProperty(this, "hdel", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HDelCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hexists
         */
        Object.defineProperty(this, "hexists", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HExistsCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hget
         */
        Object.defineProperty(this, "hget", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HGetCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hgetall
         */
        Object.defineProperty(this, "hgetall", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HGetAllCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hincrby
         */
        Object.defineProperty(this, "hincrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HIncrByCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hincrbyfloat
         */
        Object.defineProperty(this, "hincrbyfloat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HIncrByFloatCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hkeys
         */
        Object.defineProperty(this, "hkeys", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HKeysCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hlen
         */
        Object.defineProperty(this, "hlen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HLenCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hmget
         */
        Object.defineProperty(this, "hmget", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HMGetCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hmset
         */
        Object.defineProperty(this, "hmset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, kv) => this.chain(new mod_js_1.HMSetCommand([key, kv], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hrandfield
         */
        Object.defineProperty(this, "hrandfield", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, count, withValues) => this.chain(new hrandfield_js_1.HRandFieldCommand([key, count, withValues], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hscan
         */
        Object.defineProperty(this, "hscan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HScanCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hset
         */
        Object.defineProperty(this, "hset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, kv) => this.chain(new mod_js_1.HSetCommand([key, kv], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hsetnx
         */
        Object.defineProperty(this, "hsetnx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, field, value) => this.chain(new mod_js_1.HSetNXCommand([key, field, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hstrlen
         */
        Object.defineProperty(this, "hstrlen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HStrLenCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/hvals
         */
        Object.defineProperty(this, "hvals", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.HValsCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/incr
         */
        Object.defineProperty(this, "incr", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.IncrCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/incrby
         */
        Object.defineProperty(this, "incrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.IncrByCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/incrbyfloat
         */
        Object.defineProperty(this, "incrbyfloat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.IncrByFloatCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/keys
         */
        Object.defineProperty(this, "keys", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.KeysCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lindex
         */
        Object.defineProperty(this, "lindex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.LIndexCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/linsert
         */
        Object.defineProperty(this, "linsert", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, direction, pivot, value) => this.chain(new mod_js_1.LInsertCommand([key, direction, pivot, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/llen
         */
        Object.defineProperty(this, "llen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.LLenCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lmove
         */
        Object.defineProperty(this, "lmove", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.LMoveCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lpop
         */
        Object.defineProperty(this, "lpop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.LPopCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lpos
         */
        Object.defineProperty(this, "lpos", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.LPosCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lpush
         */
        Object.defineProperty(this, "lpush", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => this.chain(new mod_js_1.LPushCommand([key, ...elements], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lpushx
         */
        Object.defineProperty(this, "lpushx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => this.chain(new mod_js_1.LPushXCommand([key, ...elements], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lrange
         */
        Object.defineProperty(this, "lrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.LRangeCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lrem
         */
        Object.defineProperty(this, "lrem", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, count, value) => this.chain(new mod_js_1.LRemCommand([key, count, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/lset
         */
        Object.defineProperty(this, "lset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, index, value) => this.chain(new mod_js_1.LSetCommand([key, index, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/ltrim
         */
        Object.defineProperty(this, "ltrim", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.LTrimCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/mget
         */
        Object.defineProperty(this, "mget", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.MGetCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/mset
         */
        Object.defineProperty(this, "mset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (kv) => this.chain(new mod_js_1.MSetCommand([kv], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/msetnx
         */
        Object.defineProperty(this, "msetnx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (kv) => this.chain(new mod_js_1.MSetNXCommand([kv], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/persist
         */
        Object.defineProperty(this, "persist", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.PersistCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/pexpire
         */
        Object.defineProperty(this, "pexpire", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.PExpireCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/pexpireat
         */
        Object.defineProperty(this, "pexpireat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.PExpireAtCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/ping
         */
        Object.defineProperty(this, "ping", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (args) => this.chain(new mod_js_1.PingCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/psetex
         */
        Object.defineProperty(this, "psetex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ttl, value) => this.chain(new mod_js_1.PSetEXCommand([key, ttl, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/pttl
         */
        Object.defineProperty(this, "pttl", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.PTtlCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/publish
         */
        Object.defineProperty(this, "publish", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.PublishCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/randomkey
         */
        Object.defineProperty(this, "randomkey", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => this.chain(new mod_js_1.RandomKeyCommand(this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/rename
         */
        Object.defineProperty(this, "rename", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.RenameCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/renamenx
         */
        Object.defineProperty(this, "renamenx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.RenameNXCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/rpop
         */
        Object.defineProperty(this, "rpop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.RPopCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/rpush
         */
        Object.defineProperty(this, "rpush", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => this.chain(new mod_js_1.RPushCommand([key, ...elements], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/rpushx
         */
        Object.defineProperty(this, "rpushx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => this.chain(new mod_js_1.RPushXCommand([key, ...elements], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sadd
         */
        Object.defineProperty(this, "sadd", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...members) => this.chain(new mod_js_1.SAddCommand([key, ...members], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/scan
         */
        Object.defineProperty(this, "scan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ScanCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/scard
         */
        Object.defineProperty(this, "scard", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SCardCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/script-exists
         */
        Object.defineProperty(this, "scriptExists", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ScriptExistsCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/script-flush
         */
        Object.defineProperty(this, "scriptFlush", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ScriptFlushCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/script-load
         */
        Object.defineProperty(this, "scriptLoad", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ScriptLoadCommand(args, this.commandOptions))
        });
        /*)*
         * @see https://redis.io/commands/sdiff
         */
        Object.defineProperty(this, "sdiff", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SDiffCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sdiffstore
         */
        Object.defineProperty(this, "sdiffstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SDiffStoreCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/set
         */
        Object.defineProperty(this, "set", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, value, opts) => this.chain(new mod_js_1.SetCommand([key, value, opts], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/setbit
         */
        Object.defineProperty(this, "setbit", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SetBitCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/setex
         */
        Object.defineProperty(this, "setex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ttl, value) => this.chain(new mod_js_1.SetExCommand([key, ttl, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/setnx
         */
        Object.defineProperty(this, "setnx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, value) => this.chain(new mod_js_1.SetNxCommand([key, value], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/setrange
         */
        Object.defineProperty(this, "setrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SetRangeCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sinter
         */
        Object.defineProperty(this, "sinter", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SInterCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sinterstore
         */
        Object.defineProperty(this, "sinterstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SInterStoreCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sismember
         */
        Object.defineProperty(this, "sismember", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => this.chain(new mod_js_1.SIsMemberCommand([key, member], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/smembers
         */
        Object.defineProperty(this, "smembers", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SMembersCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/smismember
         */
        Object.defineProperty(this, "smismember", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, members) => this.chain(new mod_js_1.SMIsMemberCommand([key, members], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/smove
         */
        Object.defineProperty(this, "smove", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (source, destination, member) => this.chain(new mod_js_1.SMoveCommand([source, destination, member], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/spop
         */
        Object.defineProperty(this, "spop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SPopCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/srandmember
         */
        Object.defineProperty(this, "srandmember", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SRandMemberCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/srem
         */
        Object.defineProperty(this, "srem", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...members) => this.chain(new mod_js_1.SRemCommand([key, ...members], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sscan
         */
        Object.defineProperty(this, "sscan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SScanCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/strlen
         */
        Object.defineProperty(this, "strlen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.StrLenCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sunion
         */
        Object.defineProperty(this, "sunion", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SUnionCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/sunionstore
         */
        Object.defineProperty(this, "sunionstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.SUnionStoreCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/time
         */
        Object.defineProperty(this, "time", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => this.chain(new mod_js_1.TimeCommand(this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/touch
         */
        Object.defineProperty(this, "touch", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.TouchCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/ttl
         */
        Object.defineProperty(this, "ttl", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.TtlCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/type
         */
        Object.defineProperty(this, "type", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.TypeCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/unlink
         */
        Object.defineProperty(this, "unlink", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.UnlinkCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zadd
         */
        Object.defineProperty(this, "zadd", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => {
                if ("score" in args[1]) {
                    return this.chain(new mod_js_1.ZAddCommand([args[0], args[1], ...args.slice(2)], this.commandOptions));
                }
                return this.chain(new mod_js_1.ZAddCommand([args[0], args[1], ...args.slice(2)], this.commandOptions));
            }
        });
        /**
         * @see https://redis.io/commands/zcard
         */
        Object.defineProperty(this, "zcard", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZCardCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zcount
         */
        Object.defineProperty(this, "zcount", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZCountCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zincrby
         */
        Object.defineProperty(this, "zincrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, increment, member) => this.chain(new mod_js_1.ZIncrByCommand([key, increment, member], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zinterstore
         */
        Object.defineProperty(this, "zinterstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZInterStoreCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zlexcount
         */
        Object.defineProperty(this, "zlexcount", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZLexCountCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zmscore
         */
        Object.defineProperty(this, "zmscore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new zmscore_js_1.ZMScoreCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zpopmax
         */
        Object.defineProperty(this, "zpopmax", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZPopMaxCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zpopmin
         */
        Object.defineProperty(this, "zpopmin", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZPopMinCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zrange
         */
        Object.defineProperty(this, "zrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZRangeCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zrank
         */
        Object.defineProperty(this, "zrank", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => this.chain(new mod_js_1.ZRankCommand([key, member], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zrem
         */
        Object.defineProperty(this, "zrem", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...members) => this.chain(new mod_js_1.ZRemCommand([key, ...members], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zremrangebylex
         */
        Object.defineProperty(this, "zremrangebylex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZRemRangeByLexCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zremrangebyrank
         */
        Object.defineProperty(this, "zremrangebyrank", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZRemRangeByRankCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zremrangebyscore
         */
        Object.defineProperty(this, "zremrangebyscore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZRemRangeByScoreCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zrevrank
         */
        Object.defineProperty(this, "zrevrank", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => this.chain(new mod_js_1.ZRevRankCommand([key, member], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zscan
         */
        Object.defineProperty(this, "zscan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZScanCommand(args, this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zscore
         */
        Object.defineProperty(this, "zscore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => this.chain(new mod_js_1.ZScoreCommand([key, member], this.commandOptions))
        });
        /**
         * @see https://redis.io/commands/zunionstore
         */
        Object.defineProperty(this, "zunionstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => this.chain(new mod_js_1.ZUnionStoreCommand(args, this.commandOptions))
        });
        this.client = opts.client;
        this.commands = []; // the TCommands generic in the class definition is only used for carrying through chained command types and should never be explicitly set when instantiating the class
        this.commandOptions = opts.commandOptions;
        this.multiExec = opts.multiExec ?? false;
    }
    /**
     * Pushes a command into the pipeline and returns a chainable instance of the
     * pipeline
     */
    chain(command) {
        this.commands.push(command);
        return this; // TS thinks we're returning Pipeline<[]> here, because we're not creating a new instance of the class, hence the cast
    }
    /**
     * @see https://redis.io/commands/?group=json
     */
    get json() {
        return {
            /**
             * @see https://redis.io/commands/json.arrappend
             */
            arrappend: (...args) => this.chain(new mod_js_1.JsonArrAppendCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.arrindex
             */
            arrindex: (...args) => this.chain(new mod_js_1.JsonArrIndexCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.arrinsert
             */
            arrinsert: (...args) => this.chain(new mod_js_1.JsonArrInsertCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.arrlen
             */
            arrlen: (...args) => this.chain(new mod_js_1.JsonArrLenCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.arrpop
             */
            arrpop: (...args) => this.chain(new mod_js_1.JsonArrPopCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.arrtrim
             */
            arrtrim: (...args) => this.chain(new mod_js_1.JsonArrTrimCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.clear
             */
            clear: (...args) => this.chain(new mod_js_1.JsonClearCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.del
             */
            del: (...args) => this.chain(new mod_js_1.JsonDelCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.forget
             */
            forget: (...args) => this.chain(new mod_js_1.JsonForgetCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.get
             */
            get: (...args) => this.chain(new mod_js_1.JsonGetCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.mget
             */
            mget: (...args) => this.chain(new mod_js_1.JsonMGetCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.numincrby
             */
            numincrby: (...args) => this.chain(new mod_js_1.JsonNumIncrByCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.nummultby
             */
            nummultby: (...args) => this.chain(new mod_js_1.JsonNumMultByCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.objkeys
             */
            objkeys: (...args) => this.chain(new mod_js_1.JsonObjKeysCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.objlen
             */
            objlen: (...args) => this.chain(new mod_js_1.JsonObjLenCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.resp
             */
            resp: (...args) => this.chain(new mod_js_1.JsonRespCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.set
             */
            set: (...args) => this.chain(new mod_js_1.JsonSetCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.strappend
             */
            strappend: (...args) => this.chain(new mod_js_1.JsonStrAppendCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.strlen
             */
            strlen: (...args) => this.chain(new mod_js_1.JsonStrLenCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.toggle
             */
            toggle: (...args) => this.chain(new mod_js_1.JsonToggleCommand(args, this.commandOptions)),
            /**
             * @see https://redis.io/commands/json.type
             */
            type: (...args) => this.chain(new mod_js_1.JsonTypeCommand(args, this.commandOptions)),
        };
    }
}
exports.Pipeline = Pipeline;


/***/ }),

/***/ 8691:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.Redis = void 0;
const mod_js_1 = __nccwpck_require__(9899);
const pipeline_js_1 = __nccwpck_require__(262);
const script_js_1 = __nccwpck_require__(4411);
const zmscore_js_1 = __nccwpck_require__(6475);
const zdiffstore_js_1 = __nccwpck_require__(760);
/**
 * Serverless redis client for upstash.
 */
class Redis {
    /**
     * Create a new redis client
     *
     * @example
     * ```typescript
     * const redis = new Redis({
     *  url: "<UPSTASH_REDIS_REST_URL>",
     *  token: "<UPSTASH_REDIS_REST_TOKEN>",
     * });
     * ```
     */
    constructor(client, opts) {
        Object.defineProperty(this, "client", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "opts", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "enableTelemetry", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        /**
         * Wrap a new middleware around the HTTP client.
         */
        Object.defineProperty(this, "use", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (middleware) => {
                const makeRequest = this.client.request.bind(this.client);
                this.client.request = (req) => middleware(req, makeRequest);
            }
        });
        /**
         * Technically this is not private, we can hide it from intellisense by doing this
         */
        Object.defineProperty(this, "addTelemetry", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (telemetry) => {
                if (!this.enableTelemetry) {
                    return;
                }
                try {
                    // @ts-ignore - The `Requester` interface does not know about this method but it will be there
                    // as long as the user uses the standard HttpClient
                    this.client.mergeTelemetry(telemetry);
                }
                catch {
                    // ignore
                }
            }
        });
        /**
         * Create a new pipeline that allows you to send requests in bulk.
         *
         * @see {@link Pipeline}
         */
        Object.defineProperty(this, "pipeline", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => new pipeline_js_1.Pipeline({
                client: this.client,
                commandOptions: this.opts,
                multiExec: false,
            })
        });
        /**
         * Create a new transaction to allow executing multiple steps atomically.
         *
         * All the commands in a transaction are serialized and executed sequentially. A request sent by
         * another client will never be served in the middle of the execution of a Redis Transaction. This
         * guarantees that the commands are executed as a single isolated operation.
         *
         * @see {@link Pipeline}
         */
        Object.defineProperty(this, "multi", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => new pipeline_js_1.Pipeline({
                client: this.client,
                commandOptions: this.opts,
                multiExec: true,
            })
        });
        /**
         * @see https://redis.io/commands/append
         */
        Object.defineProperty(this, "append", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.AppendCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/bitcount
         */
        Object.defineProperty(this, "bitcount", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.BitCountCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/bitop
         */
        Object.defineProperty(this, "bitop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (op, destinationKey, sourceKey, ...sourceKeys) => new mod_js_1.BitOpCommand([op, destinationKey, sourceKey, ...sourceKeys], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/bitpos
         */
        Object.defineProperty(this, "bitpos", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.BitPosCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/dbsize
         */
        Object.defineProperty(this, "dbsize", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => new mod_js_1.DBSizeCommand(this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/decr
         */
        Object.defineProperty(this, "decr", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.DecrCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/decrby
         */
        Object.defineProperty(this, "decrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.DecrByCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/del
         */
        Object.defineProperty(this, "del", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.DelCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/echo
         */
        Object.defineProperty(this, "echo", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.EchoCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/eval
         */
        Object.defineProperty(this, "eval", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.EvalCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/evalsha
         */
        Object.defineProperty(this, "evalsha", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.EvalshaCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/exists
         */
        Object.defineProperty(this, "exists", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ExistsCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/expire
         */
        Object.defineProperty(this, "expire", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ExpireCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/expireat
         */
        Object.defineProperty(this, "expireat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ExpireAtCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/flushall
         */
        Object.defineProperty(this, "flushall", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (args) => new mod_js_1.FlushAllCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/flushdb
         */
        Object.defineProperty(this, "flushdb", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.FlushDBCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/get
         */
        Object.defineProperty(this, "get", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.GetCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/getbit
         */
        Object.defineProperty(this, "getbit", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.GetBitCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/getdel
         */
        Object.defineProperty(this, "getdel", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.GetDelCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/getrange
         */
        Object.defineProperty(this, "getrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.GetRangeCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/getset
         */
        Object.defineProperty(this, "getset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, value) => new mod_js_1.GetSetCommand([key, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hdel
         */
        Object.defineProperty(this, "hdel", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HDelCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hexists
         */
        Object.defineProperty(this, "hexists", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HExistsCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hget
         */
        Object.defineProperty(this, "hget", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HGetCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hgetall
         */
        Object.defineProperty(this, "hgetall", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HGetAllCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hincrby
         */
        Object.defineProperty(this, "hincrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HIncrByCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hincrbyfloat
         */
        Object.defineProperty(this, "hincrbyfloat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HIncrByFloatCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hkeys
         */
        Object.defineProperty(this, "hkeys", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HKeysCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hlen
         */
        Object.defineProperty(this, "hlen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HLenCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hmget
         */
        Object.defineProperty(this, "hmget", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HMGetCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hmset
         */
        Object.defineProperty(this, "hmset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, kv) => new mod_js_1.HMSetCommand([key, kv], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hrandfield
         */
        Object.defineProperty(this, "hrandfield", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, count, withValues) => new mod_js_1.HRandFieldCommand([key, count, withValues], this.opts)
                .exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hscan
         */
        Object.defineProperty(this, "hscan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HScanCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hset
         */
        Object.defineProperty(this, "hset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, kv) => new mod_js_1.HSetCommand([key, kv], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hsetnx
         */
        Object.defineProperty(this, "hsetnx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, field, value) => new mod_js_1.HSetNXCommand([key, field, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hstrlen
         */
        Object.defineProperty(this, "hstrlen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HStrLenCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/hvals
         */
        Object.defineProperty(this, "hvals", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.HValsCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/incr
         */
        Object.defineProperty(this, "incr", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.IncrCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/incrby
         */
        Object.defineProperty(this, "incrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.IncrByCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/incrbyfloat
         */
        Object.defineProperty(this, "incrbyfloat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.IncrByFloatCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/keys
         */
        Object.defineProperty(this, "keys", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.KeysCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lindex
         */
        Object.defineProperty(this, "lindex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.LIndexCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/linsert
         */
        Object.defineProperty(this, "linsert", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, direction, pivot, value) => new mod_js_1.LInsertCommand([key, direction, pivot, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/llen
         */
        Object.defineProperty(this, "llen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.LLenCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lmove
         */
        Object.defineProperty(this, "lmove", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.LMoveCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lpop
         */
        Object.defineProperty(this, "lpop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.LPopCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lpos
         */
        Object.defineProperty(this, "lpos", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.LPosCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lpush
         */
        Object.defineProperty(this, "lpush", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => new mod_js_1.LPushCommand([key, ...elements], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lpushx
         */
        Object.defineProperty(this, "lpushx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => new mod_js_1.LPushXCommand([key, ...elements], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lrange
         */
        Object.defineProperty(this, "lrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.LRangeCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lrem
         */
        Object.defineProperty(this, "lrem", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, count, value) => new mod_js_1.LRemCommand([key, count, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/lset
         */
        Object.defineProperty(this, "lset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, index, value) => new mod_js_1.LSetCommand([key, index, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/ltrim
         */
        Object.defineProperty(this, "ltrim", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.LTrimCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/mget
         */
        Object.defineProperty(this, "mget", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.MGetCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/mset
         */
        Object.defineProperty(this, "mset", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (kv) => new mod_js_1.MSetCommand([kv], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/msetnx
         */
        Object.defineProperty(this, "msetnx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (kv) => new mod_js_1.MSetNXCommand([kv], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/persist
         */
        Object.defineProperty(this, "persist", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.PersistCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/pexpire
         */
        Object.defineProperty(this, "pexpire", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.PExpireCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/pexpireat
         */
        Object.defineProperty(this, "pexpireat", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.PExpireAtCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/ping
         */
        Object.defineProperty(this, "ping", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (args) => new mod_js_1.PingCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/psetex
         */
        Object.defineProperty(this, "psetex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ttl, value) => new mod_js_1.PSetEXCommand([key, ttl, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/pttl
         */
        Object.defineProperty(this, "pttl", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.PTtlCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/publish
         */
        Object.defineProperty(this, "publish", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.PublishCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/randomkey
         */
        Object.defineProperty(this, "randomkey", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => new mod_js_1.RandomKeyCommand().exec(this.client)
        });
        /**
         * @see https://redis.io/commands/rename
         */
        Object.defineProperty(this, "rename", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.RenameCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/renamenx
         */
        Object.defineProperty(this, "renamenx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.RenameNXCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/rpop
         */
        Object.defineProperty(this, "rpop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.RPopCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/rpush
         */
        Object.defineProperty(this, "rpush", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => new mod_js_1.RPushCommand([key, ...elements], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/rpushx
         */
        Object.defineProperty(this, "rpushx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...elements) => new mod_js_1.RPushXCommand([key, ...elements], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sadd
         */
        Object.defineProperty(this, "sadd", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...members) => new mod_js_1.SAddCommand([key, ...members], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/scan
         */
        Object.defineProperty(this, "scan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ScanCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/scard
         */
        Object.defineProperty(this, "scard", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SCardCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/script-exists
         */
        Object.defineProperty(this, "scriptExists", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ScriptExistsCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/script-flush
         */
        Object.defineProperty(this, "scriptFlush", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ScriptFlushCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/script-load
         */
        Object.defineProperty(this, "scriptLoad", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ScriptLoadCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sdiff
         */
        Object.defineProperty(this, "sdiff", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SDiffCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sdiffstore
         */
        Object.defineProperty(this, "sdiffstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SDiffStoreCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/set
         */
        Object.defineProperty(this, "set", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, value, opts) => new mod_js_1.SetCommand([key, value, opts], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/setbit
         */
        Object.defineProperty(this, "setbit", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SetBitCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/setex
         */
        Object.defineProperty(this, "setex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ttl, value) => new mod_js_1.SetExCommand([key, ttl, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/setnx
         */
        Object.defineProperty(this, "setnx", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, value) => new mod_js_1.SetNxCommand([key, value], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/setrange
         */
        Object.defineProperty(this, "setrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SetRangeCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sinter
         */
        Object.defineProperty(this, "sinter", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SInterCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sinterstore
         */
        Object.defineProperty(this, "sinterstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SInterStoreCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sismember
         */
        Object.defineProperty(this, "sismember", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => new mod_js_1.SIsMemberCommand([key, member], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/smismember
         */
        Object.defineProperty(this, "smismember", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, members) => new mod_js_1.SMIsMemberCommand([key, members], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/smembers
         */
        Object.defineProperty(this, "smembers", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SMembersCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/smove
         */
        Object.defineProperty(this, "smove", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (source, destination, member) => new mod_js_1.SMoveCommand([source, destination, member], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/spop
         */
        Object.defineProperty(this, "spop", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SPopCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/srandmember
         */
        Object.defineProperty(this, "srandmember", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SRandMemberCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/srem
         */
        Object.defineProperty(this, "srem", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...members) => new mod_js_1.SRemCommand([key, ...members], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sscan
         */
        Object.defineProperty(this, "sscan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SScanCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/strlen
         */
        Object.defineProperty(this, "strlen", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.StrLenCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sunion
         */
        Object.defineProperty(this, "sunion", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SUnionCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/sunionstore
         */
        Object.defineProperty(this, "sunionstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.SUnionStoreCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/time
         */
        Object.defineProperty(this, "time", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: () => new mod_js_1.TimeCommand().exec(this.client)
        });
        /**
         * @see https://redis.io/commands/touch
         */
        Object.defineProperty(this, "touch", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.TouchCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/ttl
         */
        Object.defineProperty(this, "ttl", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.TtlCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/type
         */
        Object.defineProperty(this, "type", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.TypeCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/unlink
         */
        Object.defineProperty(this, "unlink", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.UnlinkCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zadd
         */
        Object.defineProperty(this, "zadd", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => {
                if ("score" in args[1]) {
                    return new mod_js_1.ZAddCommand([args[0], args[1], ...args.slice(2)], this.opts).exec(this.client);
                }
                return new mod_js_1.ZAddCommand([args[0], args[1], ...args.slice(2)], this.opts).exec(this.client);
            }
        });
        /**
         * @see https://redis.io/commands/zcard
         */
        Object.defineProperty(this, "zcard", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZCardCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zcount
         */
        Object.defineProperty(this, "zcount", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZCountCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zdiffstore
         */
        Object.defineProperty(this, "zdiffstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new zdiffstore_js_1.ZDiffStoreCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zincrby
         */
        Object.defineProperty(this, "zincrby", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, increment, member) => new mod_js_1.ZIncrByCommand([key, increment, member], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zinterstore
         */
        Object.defineProperty(this, "zinterstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZInterStoreCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zlexcount
         */
        Object.defineProperty(this, "zlexcount", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZLexCountCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zmscore
         */
        Object.defineProperty(this, "zmscore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new zmscore_js_1.ZMScoreCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zpopmax
         */
        Object.defineProperty(this, "zpopmax", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZPopMaxCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zpopmin
         */
        Object.defineProperty(this, "zpopmin", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZPopMinCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zrange
         */
        Object.defineProperty(this, "zrange", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZRangeCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zrank
         */
        Object.defineProperty(this, "zrank", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => new mod_js_1.ZRankCommand([key, member], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zrem
         */
        Object.defineProperty(this, "zrem", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, ...members) => new mod_js_1.ZRemCommand([key, ...members], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zremrangebylex
         */
        Object.defineProperty(this, "zremrangebylex", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZRemRangeByLexCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zremrangebyrank
         */
        Object.defineProperty(this, "zremrangebyrank", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZRemRangeByRankCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zremrangebyscore
         */
        Object.defineProperty(this, "zremrangebyscore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZRemRangeByScoreCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zrevrank
         */
        Object.defineProperty(this, "zrevrank", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => new mod_js_1.ZRevRankCommand([key, member], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zscan
         */
        Object.defineProperty(this, "zscan", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZScanCommand(args, this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zscore
         */
        Object.defineProperty(this, "zscore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (key, member) => new mod_js_1.ZScoreCommand([key, member], this.opts).exec(this.client)
        });
        /**
         * @see https://redis.io/commands/zunionstore
         */
        Object.defineProperty(this, "zunionstore", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: (...args) => new mod_js_1.ZUnionStoreCommand(args, this.opts).exec(this.client)
        });
        this.client = client;
        this.opts = opts;
        this.enableTelemetry = opts?.enableTelemetry ?? true;
    }
    get json() {
        return {
            /**
             * @see https://redis.io/commands/json.arrappend
             */
            arrappend: (...args) => new mod_js_1.JsonArrAppendCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.arrindex
             */
            arrindex: (...args) => new mod_js_1.JsonArrIndexCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.arrinsert
             */
            arrinsert: (...args) => new mod_js_1.JsonArrInsertCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.arrlen
             */
            arrlen: (...args) => new mod_js_1.JsonArrLenCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.arrpop
             */
            arrpop: (...args) => new mod_js_1.JsonArrPopCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.arrtrim
             */
            arrtrim: (...args) => new mod_js_1.JsonArrTrimCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.clear
             */
            clear: (...args) => new mod_js_1.JsonClearCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.del
             */
            del: (...args) => new mod_js_1.JsonDelCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.forget
             */
            forget: (...args) => new mod_js_1.JsonForgetCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.get
             */
            get: (...args) => new mod_js_1.JsonGetCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.mget
             */
            mget: (...args) => new mod_js_1.JsonMGetCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.numincrby
             */
            numincrby: (...args) => new mod_js_1.JsonNumIncrByCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.nummultby
             */
            nummultby: (...args) => new mod_js_1.JsonNumMultByCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.objkeys
             */
            objkeys: (...args) => new mod_js_1.JsonObjKeysCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.objlen
             */
            objlen: (...args) => new mod_js_1.JsonObjLenCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.resp
             */
            resp: (...args) => new mod_js_1.JsonRespCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.set
             */
            set: (...args) => new mod_js_1.JsonSetCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.strappend
             */
            strappend: (...args) => new mod_js_1.JsonStrAppendCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.strlen
             */
            strlen: (...args) => new mod_js_1.JsonStrLenCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.toggle
             */
            toggle: (...args) => new mod_js_1.JsonToggleCommand(args, this.opts).exec(this.client),
            /**
             * @see https://redis.io/commands/json.type
             */
            type: (...args) => new mod_js_1.JsonTypeCommand(args, this.opts).exec(this.client),
        };
    }
    createScript(script) {
        return new script_js_1.Script(this, script);
    }
}
exports.Redis = Redis;


/***/ }),

/***/ 4411:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.Script = void 0;
const mod_js_1 = __nccwpck_require__(6652);
/**
 * Creates a new script.
 *
 * Scripts offer the ability to optimistically try to execute a script without having to send the
 * entire script to the server. If the script is loaded on the server, it tries again by sending
 * the entire script. Afterwards, the script is cached on the server.
 *
 * @example
 * ```ts
 * const redis = new Redis({...})
 *
 * const script = redis.createScript<string>("return ARGV[1];")
 * const arg1 = await script.eval([], ["Hello World"])
 * assertEquals(arg1, "Hello World")
 * ```
 */
class Script {
    constructor(redis, script) {
        Object.defineProperty(this, "script", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "sha1", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        Object.defineProperty(this, "redis", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        this.redis = redis;
        this.sha1 = this.digest(script);
        this.script = script;
    }
    /**
     * Send an `EVAL` command to redis.
     */
    async eval(keys, args) {
        return await this.redis.eval(this.script, keys, args);
    }
    /**
     * Calculates the sha1 hash of the script and then calls `EVALSHA`.
     */
    async evalsha(keys, args) {
        return await this.redis.evalsha(this.sha1, keys, args);
    }
    /**
     * Optimistically try to run `EVALSHA` first.
     * If the script is not loaded in redis, it will fall back and try again with `EVAL`.
     *
     * Following calls will be able to use the cached script
     */
    async exec(keys, args) {
        const res = await this.redis.evalsha(this.sha1, keys, args).catch(async (err) => {
            if (err instanceof Error &&
                err.message.toLowerCase().includes("noscript")) {
                return await this.redis.eval(this.script, keys, args);
            }
            throw err;
        });
        return res;
    }
    /**
     * Compute the sha1 hash of the script and return its hex representation.
     */
    digest(s) {
        const hash = (0, mod_js_1.sha1)(s, "utf8", "hex");
        return typeof hash === "string" ? hash : new TextDecoder().decode(hash);
    }
}
exports.Script = Script;


/***/ }),

/***/ 3686:
/***/ ((__unused_webpack_module, exports) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.parseResponse = void 0;
function parseRecursive(obj) {
    const parsed = Array.isArray(obj)
        ? obj.map((o) => {
            try {
                return parseRecursive(o);
            }
            catch {
                return o;
            }
        })
        : JSON.parse(obj);
    /**
     * Parsing very large numbers can result in MAX_SAFE_INTEGER
     * overflow. In that case we return the number as string instead.
     */
    if (typeof parsed === "number" && parsed.toString() != obj) {
        return obj;
    }
    return parsed;
}
function parseResponse(result) {
    try {
        /**
         * Try to parse the response if possible
         */
        return parseRecursive(result);
    }
    catch {
        return result;
    }
}
exports.parseResponse = parseResponse;


/***/ }),

/***/ 4672:
/***/ (function(__unused_webpack_module, exports, __nccwpck_require__) {

"use strict";

// deno-lint-ignore-file
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || function (mod) {
    if (mod && mod.__esModule) return mod;
    var result = {};
    if (mod != null) for (var k in mod) if (k !== "default" && Object.prototype.hasOwnProperty.call(mod, k)) __createBinding(result, mod, k);
    __setModuleDefault(result, mod);
    return result;
};
Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.Redis = void 0;
const core = __importStar(__nccwpck_require__(8691));
const http_js_1 = __nccwpck_require__(2022);
const version_js_1 = __nccwpck_require__(8081);
/**
 * Workaround for nodejs 14, where atob is not included in the standardlib
 */
if (typeof atob === "undefined") {
    global.atob = function (b64) {
        return Buffer.from(b64, "base64").toString("utf-8");
    };
}
/**
 * Serverless redis client for upstash.
 */
class Redis extends core.Redis {
    constructor(configOrRequester) {
        if ("request" in configOrRequester) {
            super(configOrRequester);
            return;
        }
        if (configOrRequester.url.startsWith(" ") ||
            configOrRequester.url.endsWith(" ") ||
            /\r|\n/.test(configOrRequester.url)) {
            console.warn("The redis url contains whitespace or newline, which can cause errors!");
        }
        if (configOrRequester.token.startsWith(" ") ||
            configOrRequester.token.endsWith(" ") ||
            /\r|\n/.test(configOrRequester.token)) {
            console.warn("The redis token contains whitespace or newline, which can cause errors!");
        }
        const client = new http_js_1.HttpClient({
            baseUrl: configOrRequester.url,
            retry: configOrRequester.retry,
            headers: { authorization: `Bearer ${configOrRequester.token}` },
            agent: configOrRequester.agent,
            responseEncoding: configOrRequester.responseEncoding,
            cache: configOrRequester.cache || "no-store",
        });
        super(client, {
            automaticDeserialization: configOrRequester.automaticDeserialization,
            enableTelemetry: !process.env.UPSTASH_DISABLE_TELEMETRY,
        });
        this.addTelemetry({
            runtime: typeof EdgeRuntime === "string"
                ? "edge-light"
                : `node@${process.version}`,
            platform: process.env.VERCEL
                ? "vercel"
                : process.env.AWS_REGION
                    ? "aws"
                    : "unknown",
            sdk: `@upstash/redis@${version_js_1.VERSION}`,
        });
    }
    /**
     * Create a new Upstash Redis instance from environment variables.
     *
     * Use this to automatically load connection secrets from your environment
     * variables. For instance when using the Vercel integration.
     *
     * This tries to load `UPSTASH_REDIS_REST_URL` and `UPSTASH_REDIS_REST_TOKEN` from
     * your environment using `process.env`.
     */
    static fromEnv(config) {
        // @ts-ignore process will be defined in node
        if (typeof process?.env === "undefined") {
            throw new Error('Unable to get environment variables, `process.env` is undefined. If you are deploying to cloudflare, please import from "@upstash/redis/cloudflare" instead');
        }
        // @ts-ignore process will be defined in node
        const url = process?.env["UPSTASH_REDIS_REST_URL"];
        if (!url) {
            throw new Error("Unable to find environment variable: `UPSTASH_REDIS_REST_URL`");
        }
        // @ts-ignore process will be defined in node
        const token = process?.env["UPSTASH_REDIS_REST_TOKEN"];
        if (!token) {
            throw new Error("Unable to find environment variable: `UPSTASH_REDIS_REST_TOKEN`");
        }
        return new Redis({ ...config, url, token });
    }
}
exports.Redis = Redis;


/***/ }),

/***/ 8081:
/***/ ((__unused_webpack_module, exports) => {

"use strict";

Object.defineProperty(exports, "__esModule", ({ value: true }));
exports.VERSION = void 0;
exports.VERSION = "v1.22.0";


/***/ }),

/***/ 5076:
/***/ ((__unused_webpack_module, exports, __nccwpck_require__) => {

"use strict";
Object.defineProperty(exports, "__esModule", ({value: true}));// src/index.ts
var _redis = __nccwpck_require__(4672);
var _kv = null;
process.env.UPSTASH_DISABLE_TELEMETRY = "1";
var VercelKV = class extends _redis.Redis {
  // This API is based on https://github.com/redis/node-redis#scan-iterator which is not supported in @upstash/redis
  /**
   * Same as `scan` but returns an AsyncIterator to allow iteration via `for await`.
   */
  async *scanIterator(options) {
    let cursor = 0;
    let keys;
    do {
      [cursor, keys] = await this.scan(cursor, options);
      for (const key of keys) {
        yield key;
      }
    } while (cursor !== 0);
  }
  /**
   * Same as `hscan` but returns an AsyncIterator to allow iteration via `for await`.
   */
  async *hscanIterator(key, options) {
    let cursor = 0;
    let items;
    do {
      [cursor, items] = await this.hscan(key, cursor, options);
      for (const item of items) {
        yield item;
      }
    } while (cursor !== 0);
  }
  /**
   * Same as `sscan` but returns an AsyncIterator to allow iteration via `for await`.
   */
  async *sscanIterator(key, options) {
    let cursor = 0;
    let items;
    do {
      [cursor, items] = await this.sscan(key, cursor, options);
      for (const item of items) {
        yield item;
      }
    } while (cursor !== 0);
  }
  /**
   * Same as `zscan` but returns an AsyncIterator to allow iteration via `for await`.
   */
  async *zscanIterator(key, options) {
    let cursor = 0;
    let items;
    do {
      [cursor, items] = await this.zscan(key, cursor, options);
      for (const item of items) {
        yield item;
      }
    } while (cursor !== 0);
  }
};
function createClient(config) {
  return new VercelKV(config);
}
var src_default = new Proxy(
  {},
  {
    get(target, prop, receiver) {
      if (prop === "then" || prop === "parse") {
        return Reflect.get(target, prop, receiver);
      }
      if (!_kv) {
        if (!process.env.KV_REST_API_URL || !process.env.KV_REST_API_TOKEN) {
          throw new Error(
            "@vercel/kv: Missing required environment variables KV_REST_API_URL and KV_REST_API_TOKEN"
          );
        }
        console.warn(
          '\x1B[33m"The default export has been moved to a named export and it will be removed in version 1, change to import { kv }\x1B[0m"'
        );
        _kv = createClient({
          url: process.env.KV_REST_API_URL,
          token: process.env.KV_REST_API_TOKEN
        });
      }
      return Reflect.get(_kv, prop);
    }
  }
);
var kv = new Proxy(
  {},
  {
    get(target, prop) {
      if (!_kv) {
        if (!process.env.KV_REST_API_URL || !process.env.KV_REST_API_TOKEN) {
          throw new Error(
            "@vercel/kv: Missing required environment variables KV_REST_API_URL and KV_REST_API_TOKEN"
          );
        }
        _kv = createClient({
          url: process.env.KV_REST_API_URL,
          token: process.env.KV_REST_API_TOKEN
        });
      }
      return Reflect.get(_kv, prop);
    }
  }
);





exports.VercelKV = VercelKV; exports.createClient = createClient; exports["default"] = src_default; exports.kv = kv;
//# sourceMappingURL=index.cjs.map

/***/ })

/******/ 	});
/************************************************************************/
/******/ 	// The module cache
/******/ 	var __webpack_module_cache__ = {};
/******/ 	
/******/ 	// The require function
/******/ 	function __nccwpck_require__(moduleId) {
/******/ 		// Check if module is in cache
/******/ 		var cachedModule = __webpack_module_cache__[moduleId];
/******/ 		if (cachedModule !== undefined) {
/******/ 			return cachedModule.exports;
/******/ 		}
/******/ 		// Create a new module (and put it into the cache)
/******/ 		var module = __webpack_module_cache__[moduleId] = {
/******/ 			// no module.id needed
/******/ 			// no module.loaded needed
/******/ 			exports: {}
/******/ 		};
/******/ 	
/******/ 		// Execute the module function
/******/ 		var threw = true;
/******/ 		try {
/******/ 			__webpack_modules__[moduleId].call(module.exports, module, module.exports, __nccwpck_require__);
/******/ 			threw = false;
/******/ 		} finally {
/******/ 			if(threw) delete __webpack_module_cache__[moduleId];
/******/ 		}
/******/ 	
/******/ 		// Return the exports of the module
/******/ 		return module.exports;
/******/ 	}
/******/ 	
/************************************************************************/
/******/ 	/* webpack/runtime/compat */
/******/ 	
/******/ 	if (typeof __nccwpck_require__ !== 'undefined') __nccwpck_require__.ab = __dirname + "/";
/******/ 	
/************************************************************************/
var __webpack_exports__ = {};
// This entry need to be wrapped in an IIFE because it need to be isolated against other modules in the chunk.
(() => {
const { createClient } = __nccwpck_require__(5076);
const fs = __nccwpck_require__(3292);
const path = __nccwpck_require__(1017);

async function main() {
  try {
    const file = path.join(
      process.cwd(),
      "./test-results/main/nextjs-test-results.json"
    );

    let passingTests = "";
    let failingTests = "";
    let passCount = 0;
    let failCount = 0;

    const contents = await fs.readFile(file, "utf-8");
    const results = JSON.parse(contents);
    let { ref } = results;
    const currentDate = new Date();
    const isoString = currentDate.toISOString();
    const timestamp = isoString.slice(0, 19).replace("T", " ");

    for (const result of results.result) {
      let suitePassCount = 0;
      let suiteFailCount = 0;

      suitePassCount += result.data.numPassedTests;
      suiteFailCount += result.data.numFailedTests;

      let suiteName = result.data.testResults[0].name;
      // remove "/home/runner/work/turbo/turbo/" from the beginning of suiteName
      suiteName = suiteName.slice(30);
      if (suitePassCount > 0) {
        passingTests += `${suiteName}\n`;
      }

      if (suiteFailCount > 0) {
        failingTests += `${suiteName}\n`;
      }

      for (const assertionResult of result.data.testResults[0]
        .assertionResults) {
        let assertion = assertionResult.fullName.replaceAll("`", "\\`");
        if (assertionResult.status === "passed") {
          passingTests += `* ${assertion}\n`;
        } else if (assertionResult.status === "failed") {
          failingTests += `* ${assertion}\n`;
        }
      }

      passCount += suitePassCount;
      failCount += suiteFailCount;

      if (suitePassCount > 0) {
        passingTests += `\n`;
      }

      if (suiteFailCount > 0) {
        failingTests += `\n`;
      }
    }

    const kv = createClient({
      url: process.env.TURBOYET_KV_REST_API_URL,
      token: process.env.TURBOYET_KV_REST_API_TOKEN,
    });

    console.log("TYPEOF URL", process.env.TURBOYET_KV_REST_API_URL);
    console.log("TYPEOF TOKEN", process.env.TURBOYET_KV_REST_API_TOKEN);

    const testRun = `${ref}\t${timestamp}\t${passCount}/${
      passCount + failCount
    }`;

    console.log("TEST RESULT");
    console.log(testRun);

    await kv.rpush("test-runs-practice", testRun);
    let savedRuns = await kv.lrange("test-runs-practice", 0, -1);
    console.log("SAVED RUNS");

    await kv.set("passing-tests-practice", passingTests);
    let savedPassing = await kv.get("passing-tests-practice");
    console.log("SAVED PASSING");

    await kv.set("failing-tests-practice", failingTests);
    let savedFailing = await kv.get("failing-tests-practice");
    console.log("SAVED FAILING");
  } catch (error) {
    console.log(error);
  }
}

main();

})();

module.exports = __webpack_exports__;
/******/ })()
;
//# sourceMappingURL=index.js.map