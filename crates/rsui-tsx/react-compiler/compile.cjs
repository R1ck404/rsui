// Build-only: preserve original filenames and every module specifier.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { createRequire } = require('node:module');
const request = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const roots = request.roots.map(root => fs.realpathSync(root));
const inside = (root, file) => { const relative = path.relative(root, file); return relative === '' || (!relative.startsWith('..' + path.sep) && relative !== '..' && !path.isAbsolute(relative)); };
const dependencyPath = file => file.split(/[\\/]/).some(part => part.toLowerCase() === 'node_modules');
const externalRoots = request.explicitRoots === true ? roots.filter(dependencyPath) : [];
if (!request.scanOnly && request.explicitRoots !== true && roots.some(dependencyPath)) throw new Error('Dependency compilation requires explicit source roots');
const toolchain = process.env.RSUI_REACT_COMPILER_TOOLCHAIN || __dirname;
const requireTool = createRequire(path.join(path.resolve(toolchain), 'package.json'));
const babel = requireTool('@babel/core');
const compiler = requireTool('babel-plugin-react-compiler');
function version(name) {
  let folder = path.dirname(requireTool.resolve(name));
  for (;;) {
    const file = path.join(folder, 'package.json');
    if (fs.existsSync(file)) { const info = JSON.parse(fs.readFileSync(file, 'utf8')); if (info.name === name) return info.version; }
    const parent = path.dirname(folder); if (parent === folder) throw new Error('Cannot identify ' + name); folder = parent;
  }
}
const versions = { babel: version('@babel/core'), compiler: version('babel-plugin-react-compiler') };
if (versions.babel !== '7.29.13' || versions.compiler !== '1.0.0') throw new Error('Compiler dependencies must match crates/rsui-tsx/react-compiler/package-lock.json');
const hash = value => crypto.createHash('sha256').update(value).digest('hex');
const incompatibleApis = new Map([['react-hook-form', new Set(['useForm'])], ['@tanstack/react-table', new Set(['useReactTable'])]]);
for (const value of request.incompatibleApis || []) {
  const colon = value.lastIndexOf(':');
  if (colon <= 0 || !/^[A-Za-z_$][\w$]*$/.test(value.slice(colon + 1))) throw new Error('Expected PACKAGE:EXPORT, got ' + value);
  const library = value.slice(0, colon);
  if (!incompatibleApis.has(library)) incompatibleApis.set(library, new Set());
  incompatibleApis.get(library).add(value.slice(colon + 1));
}
const options = { target: '19', panicThreshold: 'critical_errors', sourcePolicy: 'validated-input', wrappedCallbacks: 'identity-name-v1', sourceType: 'original-module-goal', versions, incompatibleApis: [...incompatibleApis].map(([name, values]) => [name, [...values].sort()]) };
const optionsHash = hash(JSON.stringify(options));
const compilerHash = hash(fs.readFileSync(__filename));
function sourceTypeFor(input) {
  const extension = path.extname(input).toLowerCase();
  if (extension === '.mjs' || extension === '.mts') return 'module';
  if (extension === '.cjs' || extension === '.cts') return 'unambiguous';
  for (let folder = path.dirname(input);;) {
    const config = path.join(folder, 'package.json');
    if (fs.existsSync(config)) return JSON.parse(fs.readFileSync(config, 'utf8')).type === 'module' ? 'module' : 'unambiguous';
    const parent = path.dirname(folder); if (parent === folder) return 'unambiguous'; folder = parent;
  }
}
const cache = path.resolve(request.cache);
fs.mkdirSync(cache, { recursive: true });
const files = [], visited = new Set();
function walk(directory) {
  const real = fs.realpathSync(directory);
  if (visited.has(real)) return; visited.add(real);
  for (const entry of fs.readdirSync(real, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (entry.name.startsWith('.') || entry.name === 'node_modules' || entry.name === 'target') continue;
    const file = path.join(real, entry.name);
    if (entry.isSymbolicLink()) continue;
    if (entry.isDirectory()) { if (!fs.existsSync(path.join(file, 'CACHEDIR.TAG'))) walk(file); }
    else if ((request.explicitRoots === true ? /\.(tsx|jsx|ts|js|mjs|cjs|mts|cts)$/i : /\.(tsx|jsx|ts|js)$/i).test(entry.name) && !/\.d\.(ts|mts|cts)$/i.test(entry.name)) files.push(fs.realpathSync(file));
  }
}
if (request.scanOnly) {
  if (!Array.isArray(request.files)) throw new Error('Module scans require an explicit resolved files array');
  // Only files already reached by the engine resolver. A package may ship
  // unrelated tests, declarations, or source syntax its runtime never loads.
  for (const file of request.files) files.push(fs.realpathSync(file));
} else {
  for (const root of roots) walk(root);
}
const manifest = { format: 1, entry: request.entry ? fs.realpathSync(request.entry) : null, roots: request.roots, compilerHash, optionsHash, options, files: [] };
for (const input of [...new Set(files)].sort()) {
  if (!request.scanOnly && (!roots.some(root => inside(root, input)) || (dependencyPath(input) && !externalRoots.some(root => inside(root, input))))) throw new Error('Compiler input is outside validated project/explicit dependency roots: ' + input);
  const sourceText = fs.readFileSync(input, 'utf8');
  const sourceHash = hash(sourceText);
  if (request.scanOnly) {
    manifest.files.push({ input, sourceHash, ...moduleEdges(sourceText, input) });
    continue;
  }
  // The filename is an input: inference and diagnostics must retain its identity.
  const sourceType = sourceTypeFor(input);
  const key = hash(input + '\0' + sourceHash + '\0' + compilerHash + '\0' + optionsHash + '\0' + sourceType);
  const output = path.join(cache, key + '.tsx');
  const original = path.join(cache, key + '.input');
  const diagnostics = path.join(cache, key + '.json');
  let detail;
  try {
    detail = JSON.parse(fs.readFileSync(diagnostics, 'utf8'));
    if (detail.input !== input || detail.sourceHash !== sourceHash || detail.compilerHash !== compilerHash || detail.optionsHash !== optionsHash
        || hash(fs.readFileSync(original)) !== sourceHash || hash(fs.readFileSync(output)) !== detail.outputHash) detail = null;
  } catch { detail = null; }
  if (!detail) {
    const events = [], bailouts = [], adaptations = [];
    const code = compile(input, sourceText, events, bailouts, sourceType, adaptations);
    fs.writeFileSync(output, code);
    fs.writeFileSync(original, sourceText);
    detail = { input, sourceHash, compilerHash, optionsHash, sourceType, outputHash: hash(code), events, bailouts, adaptations, ...moduleEdges(code, input) };
    fs.writeFileSync(diagnostics, JSON.stringify(detail, null, 2));
  }
  manifest.files.push({ input, sourceHash, outputHash: hash(fs.readFileSync(output)), output, original, diagnostics, ...detail });
}
fs.writeFileSync(request.manifest, JSON.stringify(manifest, null, 2));
function moduleEdges(code, filename) {
  const imports = [], unresolved = [];
  babel.transformSync(code, { filename, configFile: false, babelrc: false, ast: false, code: false,
    parserOpts: { sourceType: 'unambiguous', allowReturnOutsideFunction: true, plugins: ['typescript', 'jsx'] },
    plugins: [({ types: t }) => ({ visitor: {
      ImportDeclaration(p) {
        if (p.node.importKind !== 'type' && !(p.node.specifiers.length && p.node.specifiers.every(one => one.importKind === 'type'))) imports.push({ name: p.node.source.value, import: true });
      },
      ExportNamedDeclaration(p) { if (p.node.source && p.node.exportKind !== 'type') imports.push({ name: p.node.source.value, import: true }); },
      ExportAllDeclaration(p) { if (p.node.exportKind !== 'type') imports.push({ name: p.node.source.value, import: true }); },
      CallExpression(p) {
        const node = p.node;
        const dynamic = node.callee.type === 'Import';
        const require = t.isIdentifier(node.callee, { name: 'require' }) && !p.scope.hasBinding('require');
        if (!dynamic && !require) return;
        if (t.isStringLiteral(node.arguments[0])) imports.push({ name: node.arguments[0].value, import: dynamic });
        else unresolved.push({ line: node.loc?.start.line, kind: dynamic ? 'import' : 'require' });
      }
    } })]
  });
  return { imports, unresolved };
}
function makeWrappedReactAdapter(adaptations) {
return function adapterPlugin({ types: t }) {
  function reactApi(callee) {
    if (callee.isIdentifier()) {
      const binding = callee.scope.getBinding(callee.node.name);
      return binding?.path.isImportSpecifier() && binding.path.parent.source.value === 'react'
        && ['forwardRef', 'memo'].includes(binding.path.node.imported.name);
    }
    if (!callee.isMemberExpression() || callee.node.computed || !callee.get('object').isIdentifier()) return false;
    if (!['forwardRef', 'memo'].includes(callee.node.property.name)) return false;
    const binding = callee.scope.getBinding(callee.node.object.name);
    return !!binding && (binding.path.isImportNamespaceSpecifier() || binding.path.isImportDefaultSpecifier())
      && binding.path.parent.source.value === 'react';
  }
  function builtInDefineProperty(callee, seen = new Set()) {
    if (callee.isMemberExpression()) return !callee.node.computed && callee.get('object').isIdentifier({ name: 'Object' })
      && !callee.scope.getBinding('Object') && callee.get('property').isIdentifier({ name: 'defineProperty' });
    if (!callee.isIdentifier() || seen.has(callee.node.name)) return false;
    seen.add(callee.node.name);
    const binding = callee.scope.getBinding(callee.node.name);
    return !!binding?.constant && binding.path.isVariableDeclarator()
      && builtInDefineProperty(binding.path.get('init'), seen);
  }
  function identityNameWrapper(callee) {
    if (!callee.isIdentifier()) return false;
    const binding = callee.scope.getBinding(callee.node.name);
    if (!binding?.constant) return false;
    const fn = binding.path.isVariableDeclarator() ? binding.path.get('init') : binding.path;
    if (!(fn.isFunctionDeclaration() || fn.isFunctionExpression() || fn.isArrowFunctionExpression()) || fn.node.async || fn.node.generator) return false;
    const params = fn.node.params;
    if (params.length !== 2 || !params.every(one => t.isIdentifier(one))) return false;
    let body = fn.get('body');
    if (body.isBlockStatement()) {
      if (body.node.body.length !== 1 || !body.get('body.0').isReturnStatement()) return false;
      body = body.get('body.0.argument');
    }
    if (!body.isCallExpression() || !builtInDefineProperty(body.get('callee'))) return false;
    const args = body.node.arguments;
    if (args.length !== 3 || !t.isIdentifier(args[0], { name: params[0].name }) || !t.isStringLiteral(args[1], { value: 'name' }) || !t.isObjectExpression(args[2])) return false;
    return args[2].properties.every(prop => t.isObjectProperty(prop) && !prop.computed
      && ['value', 'configurable', 'writable', 'enumerable'].includes(prop.key.name ?? prop.key.value)
      && (t.isBooleanLiteral(prop.value) || t.isStringLiteral(prop.value) || t.isIdentifier(prop.value, { name: params[1].name })));
  }
  function callback(argument) {
    let layers = 0;
    while (argument.isCallExpression() && argument.node.arguments.length === 2 && argument.get('arguments.1').isStringLiteral() && identityNameWrapper(argument.get('callee'))) {
      argument = argument.get('arguments.0'); layers++;
    }
    return layers > 0 && (argument.isFunctionExpression() || argument.isArrowFunctionExpression()) ? argument : null;
  }
  return { visitor: { Program: { exit(program) {
    program.traverse({ CallExpression(call) {
      if (!reactApi(call.get('callee')) || call.getFunctionParent() !== null) return;
      const fn = callback(call.get('arguments.0'));
      if (!fn || fn.node.async || fn.node.generator || !fn.get('body').isBlockStatement()) return;
      const directives = fn.node.body.directives ??= [];
      if (directives.some(one => ['use no memo', 'use no forget', 'use memo', 'use forget'].includes(one.value.value))) return;
      directives.unshift(t.directive(t.directiveLiteral('use memo')));
      adaptations.push({ line: fn.node.loc?.start.line, name: fn.node.id?.name ?? '<anonymous>' });
    } });
  } } } };
};
}

function compile(input, sourceText, events, bailouts, sourceType, adaptations = []) {
  const parseOptions = { filename: input, configFile: false, babelrc: false,
    parserOpts: { sourceType, allowReturnOutsideFunction: true, plugins: ['typescript', 'jsx'] } };
  const prepared = babel.transformSync(sourceText, {
    ...parseOptions, ast: true, code: false,
    plugins: [({ types: t }) => ({ visitor: { Program(program) {
      const marked = new WeakSet();
      program.traverse({ CallExpression(p) {
        const callee = p.node.callee;
        let local, exported;
        if (t.isIdentifier(callee)) local = callee.name;
        else if (t.isMemberExpression(callee) && t.isIdentifier(callee.object)) {
          local = callee.object.name;
          if (!callee.computed && t.isIdentifier(callee.property)) exported = callee.property.name;
          else if (callee.computed && t.isStringLiteral(callee.property)) exported = callee.property.value;
          else return;
        } else return;
        const binding = p.scope.getBinding(local);
        if (!binding) return;
        const specifier = binding.path;
        let library;
        if (specifier.isImportSpecifier() && exported === undefined) {
          exported = specifier.node.imported.name ?? specifier.node.imported.value;
          library = specifier.parent.source.value;
        } else if (specifier.isImportDefaultSpecifier() && exported === undefined) {
          exported = 'default';
          library = specifier.parent.source.value;
        } else if (specifier.isImportNamespaceSpecifier() && exported !== undefined) {
          library = specifier.parent.source.value;
        } else return;
        if (!incompatibleApis.get(library)?.has(exported)) return;
        const owner = p.getFunctionParent();
        if (!owner || marked.has(owner.node)) return;
        if (t.isArrowFunctionExpression(owner.node) && !t.isBlockStatement(owner.node.body)) {
          owner.node.body = t.blockStatement([t.returnStatement(owner.node.body)]);
        }
        if (!t.isBlockStatement(owner.node.body)) return;
        marked.add(owner.node);
        const directives = owner.node.body.directives ??= [];
        if (!directives.some(directive => directive.value.value === 'use no memo')) {
          directives.unshift(t.directive(t.directiveLiteral('use no memo')));
        }
        bailouts.push({ library, exported, local, function: owner.node.id?.name ?? owner.parentPath.node.id?.name ?? '<anonymous>',
          line: owner.node.loc?.start.line, reason: 'Configured incompatible API: mutable return values are not proven safe to memoize' });
      } });
    } } }), makeWrappedReactAdapter(adaptations)],
  });
  const result = babel.transformFromAstSync(prepared.ast, sourceText, {
    filename: input, configFile: false, babelrc: false,
    parserOpts: { sourceType, allowReturnOutsideFunction: true, plugins: ['typescript', 'jsx'] },
    plugins: [[compiler, { target: '19', sources: filename => filename === input, panicThreshold: 'critical_errors', logger: { logEvent(filename, event) {
      events.push({filename, ...event});
    } } }]],
  });
  if (!result || typeof result.code !== "string") throw new Error("Compiler produced no source for " + input);
  return result.code;
}
