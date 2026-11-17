const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const root = path.resolve(__dirname, '../..');
const pilot = path.join(root, 'target/performance-research/compiler-toolchain');
const env = { ...process.env };
if (!env.RSUI_REACT_COMPILER_TOOLCHAIN && !fs.existsSync(path.join(__dirname, 'node_modules')) && fs.existsSync(path.join(pilot, 'node_modules'))) env.RSUI_REACT_COMPILER_TOOLCHAIN = pilot;
function fixture(name) {
  const folder = fs.mkdtempSync(path.join(root, 'target', 'compiler-' + name + '-'));
  const project = path.join(folder, 'project');
  fs.mkdirSync(project); return { folder, project };
}
function run(f, extra = {}) {
  const request = { entry: path.join(f.project, 'App.tsx'), roots: [f.project], cache: path.join(f.folder, 'cache'), manifest: path.join(f.folder, 'manifest.json'), ...extra };
  const file = path.join(f.folder, 'request.json');
  fs.writeFileSync(file, JSON.stringify(request));
  const child = spawnSync(process.execPath, [path.join(__dirname, 'compile.cjs'), file], { env, encoding: 'utf8' });
  return { child, manifest: child.status === 0 ? JSON.parse(fs.readFileSync(request.manifest)) : null };
}
test('original filenames and module forms are preserved; hashes protect cache reuse', () => {
  const f = fixture('paths');
  const source = `import React from 'react';
import { value } from './barrel';
export { other } from '@/other';
export * from './exported';
const lazy = () => import('./lazy');
const filename = import.meta.url;
const sibling = require('./common');
export default function App() { return <text>{value + filename + sibling}</text>; }`;
  fs.writeFileSync(path.join(f.project, 'App.tsx'), source);
  fs.mkdirSync(path.join(f.project, 'node_modules'));
  fs.writeFileSync(path.join(f.project, 'node_modules', 'ignored.js'), 'invalid syntax here');
  fs.mkdirSync(path.join(f.project, '.private'));
  fs.writeFileSync(path.join(f.project, '.private', 'ignored.tsx'), 'invalid syntax here');
  let { child, manifest } = run(f);
  assert.equal(child.status, 0, child.stderr);
  assert.equal(manifest.files.length, 1);
  const file = manifest.files[0];
  assert.equal(file.input, fs.realpathSync(path.join(f.project, 'App.tsx')));
  const code = fs.readFileSync(file.output, 'utf8');
  for (const name of ['./barrel', '@/other', './exported', './lazy', './common']) assert.ok(code.includes(name), name);
  assert.ok(code.includes('import.meta.url'));
  assert.ok(code.includes('react/compiler-runtime'));
  assert.equal(fs.readFileSync(file.input, 'utf8'), source);
  for (const name of ['sourceHash', 'outputHash', 'compilerHash', 'optionsHash']) assert.match(file[name] || manifest[name], /^[0-9a-f]{64}$/);
  const mtime = fs.statSync(file.output).mtimeMs;
  ({ child, manifest } = run(f));
  assert.equal(child.status, 0, child.stderr);
  assert.equal(fs.statSync(file.output).mtimeMs, mtime);
  fs.writeFileSync(file.output, 'tampered output');
  ({ child } = run(f));
  assert.equal(child.status, 0, child.stderr);
  assert.equal(fs.readFileSync(file.output, 'utf8'), code);
});
test('configured incompatible imported APIs bail out only their bound callers', () => {
  const f = fixture('bailouts');
  fs.writeFileSync(path.join(f.project, 'App.tsx'), `
import React from 'react';
import { useForm as createForm } from 'react-hook-form';
import * as mutable from 'mutable-library';
import { useReactTable as makeTable } from '@tanstack/react-table';
export function Form() { const f = createForm(); return <text>{f.value}</text>; }
export function Another() { const f = mutable.useMutable(); return <text>{f.value}</text>; }
export function Shadow(createForm) { const f = createForm(); return <text>{f.value}</text>; }
export function Table() { const table = makeTable({}); return <text>{table.getRowModel().rows.length}</text>; }
export default function App() { return <Form />; }
`);
  const { child, manifest } = run(f, { incompatibleApis: ['mutable-library:useMutable'] });
  assert.equal(child.status, 0, child.stderr);
  const bailouts = manifest.files[0].bailouts;
  assert.deepEqual(bailouts.map(one => one.function).sort(), ['Another', 'Form', 'Table']);
  assert.ok(fs.readFileSync(manifest.files[0].output, 'utf8').includes('use no memo'));
});
test('CommonJS module kind remains CommonJS when the compiler adds its runtime helper', () => {
  const f = fixture('commonjs');
  fs.writeFileSync(path.join(f.project, 'App.tsx'), `const React = require('react');
function App(props) { return <text>{props.value}</text>; }
module.exports = App;`);
  const { child, manifest } = run(f);
  assert.equal(child.status, 0, child.stderr);
  const code = fs.readFileSync(manifest.files[0].output, 'utf8');
  assert.ok(code.includes('module.exports'));
  assert.ok(code.includes('react/compiler-runtime'));
  assert.ok(!/^import /m.test(code), code);
});
test('package module goal is respected and invalidates cached compiler output', () => {
  const f = fixture('module-goal');
  fs.writeFileSync(path.join(f.project, 'App.tsx'), 'function App(props) { return <text>{props.value}</text>; } globalThis.App = App;');
  fs.writeFileSync(path.join(f.project, 'package.json'), '{"type":"module"}');
  let { child, manifest } = run(f);
  assert.equal(child.status, 0, child.stderr);
  const moduleFile = manifest.files[0];
  assert.match(fs.readFileSync(moduleFile.output, 'utf8'), /^import /m);
  fs.writeFileSync(path.join(f.project, 'package.json'), '{"type":"commonjs"}');
  ({ child, manifest } = run(f));
  assert.equal(child.status, 0, child.stderr);
  assert.notEqual(manifest.files[0].output, moduleFile.output);
  assert.ok(!/^import /m.test(fs.readFileSync(manifest.files[0].output, 'utf8')));
});
test('syntax and configuration failures never publish a successful manifest', () => {
  const f = fixture('errors');
  fs.writeFileSync(path.join(f.project, 'App.tsx'), 'export default function App( { invalid');
  let { child, manifest } = run(f);
  assert.notEqual(child.status, 0);
  assert.equal(manifest, null);
  fs.writeFileSync(path.join(f.project, 'App.tsx'), 'export default function App() { return <text>ok</text>; }');
  ({ child } = run(f, { incompatibleApis: ['bad value'] }));
  assert.notEqual(child.status, 0);
  assert.match(child.stderr, /PACKAGE:EXPORT/);
});
test('module graph scans touch only explicit reachable files', () => {
  const f = fixture('declarations');
  fs.writeFileSync(path.join(f.project, 'App.tsx'), 'export default function App() { return <text>ok</text>; }');
  for (const extension of ['ts', 'mts', 'cts']) fs.writeFileSync(path.join(f.project, 'types.d.' + extension), 'export const value: MissingDeclarationOnlyType;');
  fs.writeFileSync(path.join(f.project, 'unused-decorated-test.tsx'), 'class Unused { @Something() value; }');
  const { child, manifest } = run(f, { scanOnly: true, files: [path.join(f.project, 'App.tsx')] });
  assert.equal(child.status, 0, child.stderr);
  assert.equal(manifest.files.length, 1);
  assert.ok(manifest.files[0].input.endsWith('App.tsx'));
});

function dependencyFixture(name, source) {
  const f = fixture(name);
  fs.writeFileSync(path.join(f.project, 'App.tsx'), 'export default function App() { return <text>app</text>; }');
  f.dist = path.join(f.project, 'node_modules', 'pure-components', 'dist');
  fs.mkdirSync(f.dist, { recursive: true });
  fs.writeFileSync(path.join(f.dist, 'index.mjs'), source);
  return f;
}
const nameWrapper = `const define = Object.defineProperty;
const named = (fn, label) => define(fn, 'name', { value: label, configurable: true });`;
test('only explicit dependency roots compile package formats and emit real memo caches', () => {
  const f = dependencyFixture('explicit-package', `import * as R from 'react';
${nameWrapper}
export const Wrapped = R.forwardRef(named(function Wrapped(props, ref) {
  const [value] = R.useState(0); return <text>{props.label + value}</text>;
}, 'Friendly'));`);
  fs.writeFileSync(path.join(f.dist, 'common.cjs'), "const React = require('react'); function App(props) { return <text>{props.label}</text>; } module.exports = App;");
  fs.writeFileSync(path.join(f.dist, 'typed.mts'), "import * as React from 'react'; export function useTyped(value: number) { const [n] = React.useState(value); return {n}; }");
  fs.writeFileSync(path.join(f.dist, 'typed.cts'), "const React = require('react'); function useTyped(value: number) { const [n] = React.useState(value); return {n}; } module.exports = useTyped;");
  fs.writeFileSync(path.join(f.dist, 'types.D.MTS'), 'invalid declaration-only input');
  let { child, manifest } = run(f);
  assert.equal(child.status, 0, child.stderr);
  assert.equal(manifest.files.length, 1, 'default scope must exclude node_modules');
  ({ child, manifest } = run(f, { roots: [f.project, f.dist] }));
  assert.notEqual(child.status, 0, 'implicit dependency roots must be rejected');
  ({ child, manifest } = run(f, { roots: [f.project, f.dist], explicitRoots: true }));
  assert.equal(child.status, 0, child.stderr);
  assert.equal(manifest.files.length, 5);
  const module = manifest.files.find(one => one.input.endsWith('index.mjs'));
  assert.equal(module.adaptations.length, 1);
  assert.ok(module.events.some(one => one.kind === 'CompileSuccess'));
  const code = fs.readFileSync(module.output, 'utf8');
  assert.match(code, /_c\(/);
  assert.ok(code.includes('R.forwardRef') && code.includes('named(function Wrapped') && code.includes('Friendly'), code);
  const common = manifest.files.find(one => one.input.endsWith('common.cjs'));
  assert.ok(!/^import /m.test(fs.readFileSync(common.output, 'utf8')));
});
test('wrapper adapter rejects spoofed React, nonidentity, reassigned and shadowed builtins', () => {
  const f = dependencyFixture('unsafe-wrappers', `import * as R from 'react';
${nameWrapper}
const React = { forwardRef: fn => fn };
export const Spoof = React.forwardRef(named(function Spoof(props) { const [n] = R.useState(0); return <text>{n}</text>; }, 'Spoof'));
const callsBack = (fn, label) => { fn(); return fn; };
export const Called = R.forwardRef(callsBack(function Called(props) { const [n] = R.useState(0); return <text>{n}</text>; }, 'Called'));
let mutableDefine = Object.defineProperty; mutableDefine = (fn) => { fn(); return fn; };
const mutableName = (fn, label) => mutableDefine(fn, 'name', { value: label });
export const Mutable = R.forwardRef(mutableName(function Mutable(props) { const [n] = R.useState(0); return <text>{n}</text>; }, 'Mutable'));
{
 const Object = { defineProperty: (fn) => { fn(); return fn; } };
 const fakeName = (fn, label) => Object.defineProperty(fn, 'name', { value: label });
 const Shadowed = R.forwardRef(fakeName(function Shadowed(props) { const [n] = R.useState(0); return <text>{n}</text>; }, 'Shadowed'));
}
`);
  const { child, manifest } = run(f, { roots: [f.project, f.dist], explicitRoots: true });
  assert.equal(child.status, 0, child.stderr);
  const module = manifest.files.find(one => one.input.endsWith('index.mjs'));
  assert.deepEqual(module.adaptations, []);
  assert.ok(!fs.readFileSync(module.output, 'utf8').includes('use memo'));
});
test('wrapped callbacks preserve opt-outs and configured incompatible API bailouts', () => {
  const f = dependencyFixture('wrapper-optouts', `import * as R from 'react';
import { useForm } from 'react-hook-form';
${nameWrapper}
export const OptOut = R.memo(named(function OptOut(props) { 'use no memo'; const [n] = R.useState(0); return <text>{n}</text>; }, 'OptOut'));
export const Form = R.forwardRef(named(function Form(props, ref) { const form = useForm(); return <text>{form.value}</text>; }, 'Form'));`);
  const { child, manifest } = run(f, { roots: [f.project, f.dist], explicitRoots: true });
  assert.equal(child.status, 0, child.stderr);
  const module = manifest.files.find(one => one.input.endsWith('index.mjs'));
  assert.deepEqual(module.adaptations, []);
  assert.equal(module.bailouts.length, 1);
  assert.ok(!/_c\(/.test(fs.readFileSync(module.output, 'utf8')));
});
test('wrapped callback annotations retain critical ref-safety validation', () => {
  const f = dependencyFixture('wrapper-refs', `import * as R from 'react';
${nameWrapper}
export const RefRead = R.forwardRef(named(function RefRead(props, ref) {
 const valueRef = R.useRef(0); return <text>{valueRef.current}</text>;
}, 'RefRead'));`);
  const { child } = run(f, { roots: [f.project, f.dist], explicitRoots: true });
  assert.notEqual(child.status, 0);
  assert.match(child.stderr, /Cannot access refs during render/);
});
