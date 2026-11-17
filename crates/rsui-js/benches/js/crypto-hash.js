// Ordinary public Hash/HMAC controls for the shared fixed-state hash core.
'use strict';
const crypto = require('crypto');
const { Buffer } = require('buffer');
// BEGIN FROZEN HASH VECTORS
const expected = {
    "md5:hash:0": "d41d8cd98f00b204e9800998ecf8427e",
    "md5:hash:64": "803aa1065fc406726673c6d1aa979979",
    "md5:hash:65536": "cc0bf7e1f110cfb29d2c33fb0e901497",
    "md5:hmac:0": "03a7d0c89cd8fde547d3eff1ec5a1472",
    "md5:hmac:64": "cb23603fe8603d31eca5f91edff547fc",
    "md5:hmac:65536": "df59522e4d7517d8b50139ebc36ee479",
    "sha1:hash:0": "da39a3ee5e6b4b0d3255bfef95601890afd80709",
    "sha1:hash:64": "c4d560855f9c091ed5bd4cbc14c45e65f63bfc18",
    "sha1:hash:65536": "1a961b255f387da27e365b9b80bf484e0c89644a",
    "sha1:hmac:0": "b41558380357eabfcc86971c08a55661a5ad1c47",
    "sha1:hmac:64": "b3b48529fadc8d84abbe60f8a5c3b9a29d4b5c29",
    "sha1:hmac:65536": "1a2ae11e9fc413903cedfea0c370686ded052c96",
    "sha256:hash:0": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "sha256:hash:64": "d801524cf855f9f15d06e4c69e380a1f7ed872736f3766e693f72b21a25b910e",
    "sha256:hash:65536": "b8d2543cb5061ac23b5a939480f1955749d9885b42f7b1eb364a32cdec982410",
    "sha256:hmac:0": "1ed17bdfa137cd9be474a8313aaeeef4ba6cccf40efd42069db67d84e21e7f92",
    "sha256:hmac:64": "9721bb70f5e5b62bed58b5e245fd823ec3f5524a94d512a6cedea1b6af2d67e7",
    "sha256:hmac:65536": "5ea2caa3bd23a5a4fdbc459b126a216b262410edce49e87652dcdd1e765c5dc6",
    "sha384:hash:0": "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b",
    "sha384:hash:64": "4f49bb70b22be0a010111689d879098a16de2b387268d9da3f94bfa075b7ac43690ab615d74eae97ffb7168f484dd87b",
    "sha384:hash:65536": "c62c7acc8d1bf1a13bde6a8293f02dfb8f115888f7f830bf00e976059bc91406cbf912d5c1746b235a36cade1df866f2",
    "sha384:hmac:0": "0a83de6c5a4af08be62332b6db5068929c0d8a7abf13dc8175d5bf2e05702d4ded95293f42d488b9ef21a93d3d346737",
    "sha384:hmac:64": "225164ad9bf0b462533d5c4176d826b09357e8b74526ae5079e8766ecdc64c1a809f0d23dbd17fedbad3be8670cfaecd",
    "sha384:hmac:65536": "27320d2691238f2c2926e9139d84bb71c0a3f5289f840833743cec7f95c352415080bbc25ee460b0ccba566817748a61",
    "sha512:hash:0": "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
    "sha512:hash:64": "248877a4ff262a55b513445a1c7b7987e6e22851cb04a6bcad170189f758b32639a3cc2f2da6e8cd6894fbb59f1c226de37fd628eb79aed11e5a18d1be5c9b19",
    "sha512:hash:65536": "d65b71b3a1f0aa0174ac54a55edec88965240b112cf4f6c7dc98b95c76520082f4a3e042b7d42242454d0a93b9c970de2b5b5caffc5a2486ab7fcb6a112504f4",
    "sha512:hmac:0": "67084dd40ad7c1e41686475dc4207ba5d2b00d615b8fe6d916b45c152ab253f77bfb9641e1bfe003e50ede2eed43c4624b339c96f8e97ae2082d42f8ad3d4b84",
    "sha512:hmac:64": "84e762f60de0c0e73b04119eedbb5b703792d00c4651af5d54791b35cbb32a99275a54abaaf39efe53882f3e780ca30a9d695e04129b75454e8dad0ce9f76aae",
    "sha512:hmac:65536": "1b920047c765ce7e0f6d923805db75617a399514264bbb4644d45cb8768070c3ce046af1e30c19f83f941e7704a1d93c6f436b0ac1eaa07c1ce49dd124726406"
};
// END FROZEN HASH VECTORS
const env = process.env;
const algorithm = env.CB_HASH || 'sha256';
const mode = env.CB_MODE || 'hash';
const length = Number(env.CB_BYTES === undefined ? 64 : env.CB_BYTES);
const calls = Number(env.CB_CALLS === undefined ? 100 : env.CB_CALLS);
if (!Number.isSafeInteger(calls) || calls <= 0) throw new Error('CB_CALLS must be a positive integer');
const key = [algorithm, mode, length].join(':');
if (!Object.prototype.hasOwnProperty.call(expected, key)) throw new Error('No frozen fixture for ' + key);
const bytes = n => {
    const result = Buffer.alloc(n);
    for (let i = 0; i < n; i++) result[i] = (i * 29 + 17) & 255;
    return result;
};
const input = bytes(length), password = bytes(8);
let checksum = 0;
function digest() {
    const state = mode === 'hash' ? crypto.createHash(algorithm) : crypto.createHmac(algorithm, password);
    const result = state.update(input).digest('hex');
    if (result !== expected[key]) throw new Error('Hash/HMAC output mismatch');
    for (let i = 0; i < result.length; i++) checksum = (Math.imul(checksum, 33) ^ result.charCodeAt(i)) >>> 0;
}
digest(); // verified warmup; fixture construction and this call are outside the warm sample
const start = performance.now();
for (let i = 0; i < calls; i++) digest();
const elapsed = performance.now() - start;
process.stderr.write(JSON.stringify({ case: key, calls, warmMs: elapsed, checksum }) + '\n');
console.log('crypto-hash ok');
