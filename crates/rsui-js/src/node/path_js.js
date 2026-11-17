'use strict';
// Node's `path` as it was written in JavaScript, which `node/path.js` still gives anything that isn't text, so that's done
// exactly as it always was. It's only compiled if that happens. What's done with text is in `builtins/host/path.rs`.
function makePath(separator, isAbsolute, splitRoot) {
    function normalizeParts(parts, keepUp) {
        const out = [];
        for (const part of parts) {
            if (part === '' || part === '.') continue;
            if (part === '..') {
                if (out.length && out[out.length - 1] !== '..') out.pop();
                else if (keepUp) out.push('..');
            } else out.push(part);
        }
        return out;
    }
    const splitter = separator === '\\' ? /[\\/]+/ : /\/+/;
    const path = {
        sep: separator,
        delimiter: separator === '\\' ? ';' : ':',
        isAbsolute,
        normalize(p) {
            if (p === '') return '.';
            const { root, rest } = splitRoot(p);
            const trailing = /[\\/]$/.test(rest) && separator === '\\' || (separator === '/' && rest.endsWith('/'));
            const parts = normalizeParts(rest.split(splitter), !root);
            let out = parts.join(separator);
            if (!out && !root) out = '.';
            if (out && trailing) out += separator;
            return root + out;
        },
        join(...parts) {
            const joined = parts.filter(part => part !== '').join(separator);
            return joined === '' ? '.' : path.normalize(joined);
        },
        resolve(...parts) {
            let resolved = '';
            for (let i = parts.length - 1; i >= 0 && !isAbsolute(resolved); i--) {
                if (parts[i]) resolved = resolved ? parts[i] + separator + resolved : parts[i];
            }
            if (!isAbsolute(resolved)) resolved = process.cwd() + separator + resolved;
            const { root, rest } = splitRoot(resolved);
            return root + normalizeParts(rest.split(splitter), false).join(separator) || root || '.';
        },
        dirname(p) {
            const { root, rest } = splitRoot(p);
            const trimmed = rest.replace(separator === '\\' ? /[\\/]+$/ : /\/+$/, '');
            const at = Math.max(trimmed.lastIndexOf('/'), separator === '\\' ? trimmed.lastIndexOf('\\') : -1);
            if (at < 0) return root || '.';
            return root + trimmed.slice(0, at);
        },
        basename(p, ext) {
            const trimmed = p.replace(separator === '\\' ? /[\\/]+$/ : /\/+$/, '');
            const at = Math.max(trimmed.lastIndexOf('/'), separator === '\\' ? trimmed.lastIndexOf('\\') : -1);
            let base = trimmed.slice(at + 1);
            if (ext && base.endsWith(ext) && base !== ext) base = base.slice(0, -ext.length);
            return base;
        },
        extname(p) {
            const base = path.basename(p);
            const at = base.lastIndexOf('.');
            return at <= 0 ? '' : base.slice(at);
        },
        relative(from, to) {
            const a = path.resolve(from).split(splitter).filter(Boolean);
            const b = path.resolve(to).split(splitter).filter(Boolean);
            let same = 0;
            while (same < a.length && same < b.length && a[same] === b[same]) same++;
            return [...a.slice(same).map(() => '..'), ...b.slice(same)].join(separator);
        },
        parse(p) {
            const { root } = splitRoot(p);
            const base = path.basename(p);
            const ext = path.extname(p);
            const dir = path.dirname(p);
            return { root, dir: dir === '.' && !p.startsWith('.') ? '' : dir, base, ext, name: ext ? base.slice(0, -ext.length) : base };
        },
        format(o) {
            const dir = o.dir || o.root;
            const base = o.base || ((o.name || '') + (o.ext || ''));
            if (!dir) return base;
            return dir === o.root ? dir + base : dir + separator + base;
        },
        toNamespacedPath(p) { return p; },
    };
    return path;
}
const posix = makePath('/', p => p.startsWith('/'), p => p.startsWith('/') ? { root: '/', rest: p.slice(1) } : { root: '', rest: p });
const win32 = makePath('\\', p => /^([a-zA-Z]:)?[\\/]/.test(p), p => {
    const found = /^([a-zA-Z]:)?[\\/]?/.exec(p)[0];
    return { root: found.replace(/\//g, '\\'), rest: p.slice(found.length) };
});
const current = process.platform === 'win32' ? win32 : posix;
module.exports = Object.assign({}, current, { posix, win32 });
