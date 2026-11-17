// What makes an app's CSS into what a browser would have been given, with the tools the app has installed, as its bundler would
// have done: its PostCSS settings, if it has some, and otherwise Tailwind 4, which is a compiler of its own rather than a PostCSS
// plugin. It runs in an engine of its own, apart from the app's, which `css_build.rs` starts for it: see there. What it's asked is
// in `__rsuiCssBuildRequest`, and what it makes goes in `__rsuiCssBuildResult`, both as JSON. `__rsuiScan` is the app's code read
// for what could be class names, which is done in Rust, as Tailwind 4 does it in Rust too.

const path = require('path');
const fs = require('fs');

const request = JSON.parse(globalThis.__rsuiCssBuildRequest);
// Where the app's tools run from, as a bundler runs them from the project's folder.
process.cwd = () => request.project;
// Every file this read, for knowing when what it made is out of date.
const read = new Set();

function readText(file) {
    read.add(file);
    return fs.readFileSync(file, 'utf8');
}

/// What a module exports by default, from what `require` gives for it.
const byDefault = (loaded) => (loaded !== null && typeof loaded === 'object' && 'default' in loaded ? loaded.default : loaded);

/// A package the app has installed, from where the CSS is, or `null`.
function installed(name, from) {
    try {
        return require.resolve(name, { paths: [from] });
    } catch {
        return null;
    }
}

/// A module named by `@plugin` or `@config`, from the folder that names it, as Node finds it.
function resolveFrom(id, base) {
    return id.startsWith('.') || path.isAbsolute(id) ? require.resolve(path.resolve(base, id)) : require.resolve(id, { paths: [base] });
}

/// The app's PostCSS settings, read the way PostCSS's own tools read them, or `null` if it has none.
function postcssSettings() {
    const loader = installed('postcss-load-config', request.project) ?? installedNear('postcss-load-config');
    if (loader === null) {
        return null;
    }
    return byDefault(require(loader))({ env: 'production', cwd: request.project }, request.base).then(
        (found) => {
            if (typeof found.file === 'string') {
                read.add(found.file);
            }
            return found;
        },
        (error) => {
            // Having none isn't something wrong; a file that's there and doesn't load is.
            if (/No PostCSS Config found/i.test(String(error?.message ?? error))) {
                return null;
            }
            throw error;
        },
    );
}

/// A package that's installed for one of the app's own packages, rather than for the app itself, as pnpm keeps them.
function installedNear(name) {
    for (const own of request.packages) {
        try {
            const found = installed(name, fs.realpathSync(path.join(request.project, 'node_modules', own)));
            if (found !== null) {
                return found;
            }
        } catch {}
    }
    return null;
}

async function withPostcss(settings) {
    const postcss = require(installed('postcss', request.project) ?? installedNear('postcss'));
    // Nothing here reads source maps, so none is made or followed, as a stylesheet's own `sourceMappingURL` would have it be.
    const result = await postcss(settings.plugins).process(request.css, { ...settings.options, from: request.from, map: false });
    for (const message of result.messages) {
        if (message.type === 'dependency' && typeof message.file === 'string') {
            read.add(message.file);
        }
    }
    return { css: result.css, built: true, sources: [] };
}

async function withTailwind(lib) {
    const { compile } = require(lib);
    const missing = [];
    const compiler = await compile(request.css, {
        base: request.base,
        from: request.from,
        loadStylesheet: async (id, base) => {
            // As Node finds a module, with a package's stylesheet being what its `exports` say for `style`.
            const file = __rsuiResolveStylesheet(id, base);
            if (file === null) {
                missing.push(id);
                // What isn't there is left out, as the rest is still worth having.
                return { path: id, base, content: '' };
            }
            return { path: file, base: path.dirname(file), content: readText(file) };
        },
        loadModule: async (id, base) => {
            const file = resolveFrom(id, base);
            read.add(file);
            return { path: file, base: path.dirname(file), module: byDefault(require(file)) };
        },
        onDependency: (file) => read.add(file),
    });
    // Nothing in it is Tailwind's, so it's left as it was, as Tailwind's own bundler plugins leave it.
    if (compiler.features === 0) {
        return { css: request.css, built: false, sources: [] };
    }
    // Where else classes are looked for, as `@source` says, besides the app's own code.
    const sources = [];
    if (compiler.root !== null && compiler.root !== 'none') {
        sources.push({ base: compiler.root.base, pattern: compiler.root.pattern, negated: false });
    }
    for (const one of compiler.sources ?? []) {
        sources.push({ base: one.base, pattern: one.pattern, negated: one.negated === true });
    }
    const candidates = __rsuiScan(JSON.stringify(sources));
    return { css: compiler.build(candidates), built: true, sources, missing };
}

async function build() {
    const settings = await postcssSettings();
    if (settings !== null) {
        return withPostcss(settings);
    }
    const lib = installed('tailwindcss', request.base);
    if (lib !== null) {
        const version = JSON.parse(readText(installed('tailwindcss/package.json', request.base))).version;
        if (Number(version.split('.')[0]) >= 4) {
            return withTailwind(lib);
        }
    }
    return { css: request.css, built: false, sources: [] };
}

build()
    .then((made) => {
        const files = [...read].filter((file) => {
            try {
                return fs.statSync(file).isFile();
            } catch {
                return false;
            }
        });
        globalThis.__rsuiCssBuildResult = JSON.stringify({ ...made, files });
    })
    .catch((error) => {
        globalThis.__rsuiCssBuildResult = JSON.stringify({ error: String(error?.stack ?? error), files: [...read] });
    });
