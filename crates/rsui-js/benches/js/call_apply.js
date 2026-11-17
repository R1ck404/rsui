// Ways of calling: call, apply, bind, the arguments object, default parameters, and calls with too few or too many arguments.
function add3(a, b, c) { return a + b + c; }
function withThis(k) { return this.base + k; }
function countArgs() { let s = 0; for (let i = 0; i < arguments.length; i++) s += arguments[i]; return s + arguments.length; }
function defaults(a, b = 2, c = a + b) { return a + b + c; }
const holder = { base: 10 };
const bound = withThis.bind(holder);
const partial = add3.bind(null, 1, 2);
let total = 0;
for (let i = 0; i < 300000; i++) {
    total += add3.call(null, i, 1, 2);
    total += add3.apply(null, [i, 1, 2]);
    total += withThis.call(holder, i & 7);
    total += bound(i & 3) + partial(i & 1);
    total += countArgs(1, 2, 3, i & 1);
    total += defaults(i & 15) + defaults(1, i & 3);
    total += add3(i) !== add3(i) ? 1 : 0;
    total += add3(1, 2, 3, 4, 5);
    total = total % 1000000007;
}
console.log('call_apply', total);
