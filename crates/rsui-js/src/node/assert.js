'use strict';
// Node's `assert`, for packages that expect it. It's only compiled if something asks for it.
const { isDeepStrictEqual, inspect } = require('util');
class AssertionError extends Error {
    constructor(options) {
        super(options.message);
        this.name = 'AssertionError';
        this.code = 'ERR_ASSERTION';
        this.actual = options.actual;
        this.expected = options.expected;
        this.operator = options.operator;
    }
}
function fail(actual, expected, message, operator) {
    if (message instanceof Error) throw message;
    throw new AssertionError({ message: message || `${inspect(actual)} ${operator} ${inspect(expected)}`, actual, expected, operator });
}
const assert = (value, message) => { if (!value) fail(value, true, message || 'The expression evaluated to a falsy value', '=='); };
Object.assign(assert, {
    AssertionError,
    ok: assert,
    equal(a, b, m) { if (a != b) fail(a, b, m, '=='); },
    notEqual(a, b, m) { if (a == b) fail(a, b, m, '!='); },
    strictEqual(a, b, m) { if (!Object.is(a, b)) fail(a, b, m, 'strictEqual'); },
    notStrictEqual(a, b, m) { if (Object.is(a, b)) fail(a, b, m, 'notStrictEqual'); },
    deepEqual(a, b, m) { if (!isDeepStrictEqual(a, b)) fail(a, b, m, 'deepEqual'); },
    deepStrictEqual(a, b, m) { if (!isDeepStrictEqual(a, b)) fail(a, b, m, 'deepStrictEqual'); },
    notDeepStrictEqual(a, b, m) { if (isDeepStrictEqual(a, b)) fail(a, b, m, 'notDeepStrictEqual'); },
    fail(m) { fail(undefined, undefined, m || 'Failed', 'fail'); },
    throws(f, expected, m) {
        try { f(); } catch (error) {
            if (typeof expected === 'function' && expected.prototype !== undefined && !(error instanceof expected)) throw error;
            if (expected instanceof RegExp && !expected.test(String(error))) throw error;
            return;
        }
        fail(undefined, expected, m || 'Missing expected exception.', 'throws');
    },
    doesNotThrow(f) { f(); },
    async rejects(p, expected, m) {
        try { await (typeof p === 'function' ? p() : p); } catch (error) { return; }
        fail(undefined, expected, m || 'Missing expected rejection.', 'rejects');
    },
    match(s, r, m) { if (!r.test(s)) fail(s, r, m, 'match'); },
});
assert.strict = assert;
module.exports = assert;
