//! Queue behavior in the embedded Node stream module, including rsui's existing scheduling.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rsui_js::Engine;

const PRELUDE: &str = r#"
const { Readable, Writable, PassThrough, pipeline } = require('stream');
function check(value, message) { if (!value) throw new Error(message); }
function equal(actual, expected) {
    check(JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual) + ' != ' + JSON.stringify(expected));
}
"#;

// File creation, Engine construction and module loading precede the timer. Timed execution
// includes compilation of the small invocation, input allocation, assertions, microtasks,
// printing and run_until_idle; this is not an isolated queue-operation or RSS measurement.
fn execute(name: &str, setup: &str, invocation: &str) -> (String, Duration) {
    let dir = std::env::temp_dir().join(format!(
        "rsui_js_node_streams_{name}_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.js");
    std::fs::write(&file, format!("{PRELUDE}\n{setup}")).unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let sink = output.clone();
    let mut engine = Engine::new();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let setup_result = engine
        .run_file(&file)
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    assert_eq!(setup_result, Ok(()));
    let start = Instant::now();
    let result = engine
        .eval(invocation, "invoke.js")
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    let elapsed = start.elapsed();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(result, Ok(()));
    let shown = output.borrow().replace("\r\n", "\n");
    (shown.trim_end().to_owned(), elapsed)
}

fn run(name: &str, source: &str) -> String {
    execute(name, source, "undefined").0
}

#[test]
fn file_streams_pull_bounded_ranges_and_survive_path_replacement() {
    assert_eq!(
        run("file-ranges", r#"
        const fs = require('fs');
        const file = require('path').join(__dirname, 'stream.bin');
        const collect = async s => { const out = []; for await (const x of s) out.push(x); return out; };
        const delay = () => new Promise(resolve => setTimeout(resolve, 8));
        (async () => {
            fs.writeFileSync(file, 'a€xyz');
            const range = fs.createReadStream(file, { start: 1, end: 3, highWaterMark: 1, encoding: 'utf8' });
            equal((await collect(range)).join(''), '€'); equal(range.bytesRead, 3);
            equal((await collect(fs.createReadStream(file, { start: 1, end: 1, encoding: 'utf8' }))).join(''), '�');
            equal((await collect(fs.createReadStream(file, { start: 4, end: 999, highWaterMark: 2 }))).map(String), ['xy','z']);
            equal(await collect(fs.createReadStream(file, { start: 999 })), []);
            equal(await collect(fs.createReadStream(file, { highWaterMark: 0 })), []);
            const empty = file + '.empty'; fs.writeFileSync(empty, '');
            equal(await collect(fs.createReadStream(empty)), []);
            let bad; try { await collect(fs.createReadStream(file + '.missing')); } catch (e) { bad = e.code; }
            equal(bad, 'ENOENT');
            for (const options of [{start:-1}, {end:-2}, {start:2,end:1}, {highWaterMark:-1}]) {
                let rejected = false; try { fs.createReadStream(file,options); } catch { rejected = true; }
                check(rejected,'invalid range');
            }
            fs.writeFileSync(file, 'abcdefghijklmnop');
            const slow = fs.createReadStream(file, { highWaterMark: 2 });
            const iter = slow[Symbol.asyncIterator]();
            equal(String((await iter.next()).value),'ab');
            await delay(); equal(slow.bytesRead,2);
            fs.renameSync(file,file+'.old'); fs.writeFileSync(file,'replacement');
            equal(String((await iter.next()).value),'cd');
            await iter.return(); await delay();
            check(slow.closed && slow.bytesRead === 4,'early iterator close');
            const paused = fs.createReadStream(file, { highWaterMark: 2 });
            const chunks = [];
            await new Promise((done, reject) => {
                paused.on('error',reject);
                paused.on('data',x=>{
                    chunks.push(String(x));
                    if (chunks.length===1) { paused.pause(); setTimeout(()=>{ if(paused.bytesRead!==2) reject(Error('read ahead')); paused.resume(); },8); }
                });
                paused.on('end',done);
            });
            equal(chunks.join(''),'replacement');
            const stopped = fs.createReadStream(file, { highWaterMark: 2 });
            await new Promise((done, reject) => {
                stopped.once('close', done);
                stopped.on('error', reject);
                stopped.on('data', () => stopped.destroy());
            });
            equal(stopped.bytesRead,2);
            console.log('file streams ok');
        })().catch(e=>console.log(e.stack));
        "#),
        "file streams ok"
    );
}

#[test]
fn readable_lengths_decoding_and_read_all() {
    assert_eq!(
        run(
            "lengths",
            r#"
        const r = new Readable({ highWaterMark: 3 });
        equal([r.push(Buffer.alloc(0)), r.readableLength], [true, 0]);
        const a = Buffer.from('ab');
        equal([r.push(a), r.readableLength, r.push('c'), r.readableLength], [true, 2, false, 3]);
        // rsui currently reads all bytes even when a size argument is supplied.
        equal(String(r.read(1)), 'abc');
        equal([r.readableLength, r.read()], [0, null]);
        r.push(a); check(r.read() === a, 'single buffer identity');
        r.unshift(Buffer.from('z')); equal(r.readableLength, 1); equal(String(r.read()), 'z');
        const d = new Readable({ encoding: 'utf8', highWaterMark: 2 });
        equal([d.push(Buffer.from([0xe2])), d.readableLength], [true, 0]);
        // The decoder's empty fragment is stored and delivered by this implementation.
        equal(d.read(), '');
        equal([d.push(Buffer.from([0x82, 0xac])), d.readableLength], [true, 1]);
        equal([d.push('x'), d.readableLength, d.read()], [false, 2, '\u20acx']);
        d.push('\ud83d\ude00'); equal(d.readableLength, 2); equal(d.read(), '\ud83d\ude00');
        const s = new Readable(); s.setEncoding('utf8'); s.push('hi');
        equal([s.readableLength, s.read(), s.readableLength], [2, 'hi', 0]);
        console.log('lengths ok');
    "#
        ),
        "lengths ok"
    );
}

#[test]
fn object_fifo_unshift_and_repeated_refill() {
    assert_eq!(
        run(
            "objects",
            r#"
        const r = new Readable({ objectMode: true, highWaterMark: 2 });
        const first = {}, second = {};
        equal([r.push(first), r.push(second), r.readableLength], [true, false, 2]);
        check(r.read() === first, 'first identity');
        r.unshift(first); check(r.read() === first, 'unshift identity');
        check(r.read() === second, 'second identity'); equal(r.readableLength, 0);
        for (let round = 0; round < 2; round++) {
            for (let i = 0; i < 3000; i++) r.push({ i });
            for (let i = 0; i < 1700; i++) equal(r.read().i, i);
            r.unshift({ i: -1 }); r.unshift({ i: -2 });
            equal([r.read().i, r.read().i, r.readableLength], [-2, -1, 1300]);
            for (let i = 1700; i < 3000; i++) equal(r.read().i, i);
            equal([r.readableLength, r.read()], [0, null]);
        }
        console.log('objects ok');
    "#
        ),
        "objects ok"
    );
}

#[test]
fn flowing_pause_and_reentrant_reads_preserve_identity_and_order() {
    assert_eq!(
        run(
            "flow",
            r#"
        const r = new Readable({ objectMode: true });
        const values = [{ i: 0 }, { i: 1 }, { i: 2 }];
        const events = [];
        values.forEach(value => r.push(value));
        r.on('data', value => {
            check(value === values[value.i], 'flow identity');
            events.push('data' + value.i + ':' + r.readableLength);
            if (value.i === 0 && events.length === 1) {
                check(r.read() === values[1], 'reentrant read');
                r.unshift(values[1]); r.push(values[0]); r.pause();
                queueMicrotask(() => { events.push('resume'); r.resume(); });
            }
            if (events.length > 4) { r.removeAllListeners('data'); r.read(); r.push(null); }
        });
        r.on('end', () => { events.push('end'); console.log(events.join(',')); });
    "#
        ),
        "data0:2,resume,data1:2,data2:1,data0:0,end"
    );
}

#[test]
fn synchronous_and_delayed_writes_keep_callback_and_drain_order() {
    assert_eq!(
        run(
            "writes",
            r#"
        (async () => {
            for (const delayed of [false, true]) {
                const events = []; let release;
                const w = new Writable({ objectMode: true, highWaterMark: 2,
                    write(value, encoding, done) {
                        events.push('write' + value);
                        if (delayed && value === 0) release = done;
                        else done();
                    },
                    final(done) { events.push('final'); done(); }
                });
                w.on('drain', () => events.push('drain'));
                w.on('finish', () => events.push('finish'));
                equal(w.write(0, () => events.push('cb0')), true);
                equal(w.write(1, () => events.push('cb1')), false);
                events.push('returned');
                if (delayed) release();
                await new Promise(resolve => w.once('drain', resolve));
                await new Promise(resolve => w.end(() => { events.push('endcb'); resolve(); }));
                equal(events, ['write0', 'returned', 'cb0', 'write1', 'cb1', 'drain', 'final', 'finish', 'endcb']);
                console.log(delayed ? 'delayed ok' : 'sync ok');
            }
            const w = new Writable({ write(chunk, encoding, done) { done(); } });
            let drains = 0; w.on('drain', () => drains++);
            w.write(Buffer.alloc(20000));
            await new Promise(resolve => w.end(resolve));
            equal(drains, 0); console.log('end suppresses drain');
        })();
    "#
        ),
        "sync ok\ndelayed ok\nend suppresses drain"
    );
}

#[test]
fn errors_pipeline_and_async_iteration() {
    assert_eq!(
        run(
            "composition",
            r#"
        (async () => {
            const events = [];
            const w = new Writable({ write(chunk, encoding, done) { done(new Error('broken')); } });
            w.on('error', e => events.push('error:' + e.message));
            const closed = new Promise(resolve => w.on('close', resolve));
            w.write('x', e => events.push('callback:' + e.message)); events.push('returned');
            await closed; equal(events, ['returned', 'callback:broken', 'error:broken']);
            let nullCode;
            try { new Writable().write(null); } catch (e) { nullCode = e.code; }
            equal(nullCode, 'ERR_STREAM_NULL_VALUES');
            const ended = new Writable(); ended.end();
            const late = [];
            const error = new Promise(resolve => ended.on('error', e => { late.push(e.code); resolve(); }));
            equal(ended.write('x', e => late.push(e.code)), false); late.push('returned');
            await error; equal(late, ['returned', 'ERR_STREAM_WRITE_AFTER_END', 'ERR_STREAM_WRITE_AFTER_END']);
            const source = Readable.from([Buffer.from('a'), Buffer.from('bc')]);
            const through = new PassThrough(); let joined = '';
            const sink = new Writable({ write(chunk, encoding, done) { joined += chunk; done(); } });
            const order = []; source.on('end', () => order.push('end')); sink.on('finish', () => order.push('finish'));
            await new Promise((resolve, reject) => pipeline(source, through, sink, e => e ? reject(e) : resolve()));
            equal(joined, 'abc'); equal(order, ['end', 'finish']);
            const r = new Readable({ objectMode: true });
            for (let i = 0; i < 3000; i++) r.push(i);
            r.push(null); let count = 0;
            for await (const value of r) { equal(value, count); count++; }
            equal(count, 3000);
            const bad = new Readable({ read() { this.destroy(new Error('read failed')); } });
            let message;
            try { for await (const value of bad) { throw new Error('unexpected data'); } } catch (e) { message = e.message; }
            equal(message, 'read failed');
            console.log('composition ok');
        })();
    "#
        ),
        "composition ok"
    );
}

#[test]
fn bytes_read_all_after_flowing_and_empty_decoded_chunks() {
    assert_eq!(
        run(
            "byte_flow",
            r#"
        const r = new Readable();
        r.push('a'); r.push('bc'); r.push('de');
        r.on('data', chunk => {
            equal(String(chunk), 'a'); equal(r.readableLength, 4);
            r.pause(); r.unshift(Buffer.from('z'));
            equal(r.readableLength, 5); equal(String(r.read()), 'zbcde');
            equal(r.readableLength, 0); r.push(null); r.resume();
        });
        const d = new Readable({ encoding: 'utf8' });
        d.push(Buffer.from([0xe2])); d.push(Buffer.from([0x82, 0xac])); d.push(null);
        const parts = []; d.on('data', chunk => parts.push([chunk, d.readableLength]));
        d.on('end', () => { equal(parts, [['', 1], ['\u20ac', 0]]); console.log('byte flow ok'); });
    "#
        ),
        "byte flow ok"
    );
}

#[test]
fn queued_writes_refill_and_accept_reentrant_callbacks() {
    assert_eq!(
        run(
            "write_refill",
            r#"
        (async () => {
            let release, count = 0, callbacks = 0;
            const w = new Writable({ objectMode: true, write(value, encoding, done) {
                equal(value, count++);
                if (value % 3000 === 0) release = done; else done();
            } });
            const callback = i => error => { check(!error, 'write error'); equal(callbacks++, i); };
            for (let round = 0; round < 2; round++) {
                for (let i = round * 3000; i < (round + 1) * 3000; i++) w.write(i, callback(i));
                const drained = new Promise(resolve => w.once('drain', resolve));
                release(); await drained;
                equal(callbacks, (round + 1) * 3000);
            }
            w.write(6000, error => {
                callback(6000)(error);
                w.write(6001, callback(6001));
                w.end();
            });
            const finished = new Promise(resolve => w.once('finish', resolve));
            release(); await finished;
            equal([count, callbacks], [6002, 6002]);
            console.log('write refill ok');
        })();
    "#
        ),
        "write refill ok"
    );
}

// White-box coverage supplements the public tests: prompt reference release and bounded
// backing slots are requirements of this optimization, not claims about process RSS.
#[test]
fn consumed_slots_are_released_and_backing_storage_shrinks() {
    assert_eq!(
        run(
            "storage",
            r#"
        const r = new Readable({ objectMode: true });
        for (let i = 0; i < 5000; i++) r.push({ i });
        const original = r._chunks._items;
        equal(r.read().i, 0); check(original[0] === undefined, 'release first slot');
        for (let i = 1; i < 5000; i++) {
            const oldItems = r._chunks._items, oldHead = r._chunks._head;
            equal(r.read().i, i);
            check(oldItems[oldHead] === undefined, 'release consumed slot before compaction');
            const live = 4999 - i;
            check(r._chunks._items.length <= Math.max(live + 1023, live * 2), 'bounded backing slots');
        }
        equal([r._chunks._items.length, r._chunks._head, r.readableLength], [0, 0, 0]);
        r.push({ i: 5 }); equal(r.read().i, 5);
        equal(r._chunks._items.length, 0);
        const small = r._chunks._items;
        for (let i = 0; i < 1000; i++) {
            r.push({ i }); equal(r.read().i, i);
            check(r._chunks._items === small, 'reuse empty small queue');
            equal([small.length, r.readableLength], [0, 0]);
        }
        const b = new Readable(); b.push('ab'); b.push('cd'); b.read();
        equal([b._chunks._items.length, b._chunks._head, b.readableLength], [0, 0, 0]);
        let first, second;
        const w = new Writable({ objectMode: true, write(value, encoding, done) {
            if (value === 0) first = done; else second = done;
        } });
        w.write(0); w.write(1); w.write(2);
        const pending = w._writing;
        first(); check(pending.every(entry => entry.chunk !== 1), 'release queued write before callback');
        second(); equal(pending.length, 0);
        second(); w.end();
        console.log('storage ok');
    "#
        ),
        "storage ok"
    );
}

const BENCHMARK: &str = r#"
globalThis.runCase = async function runCase(kind, n) {
    let count = 0, checksum = 0, callbacks = 0;
    const accept = value => { equal(value, count); checksum += value; count++; };
    if (kind === 'objects') {
        const r = new Readable({ objectMode: true });
        for (let i = 0; i < n; i++) r.push({ i });
        equal(r.readableLength, n);
        for (let i = 0; i < n; i++) accept(r.read().i);
        equal(r.readableLength, 0);
    } else if (kind === 'trickle') {
        const r = new Readable({ objectMode: true });
        for (let i = 0; i < n; i++) {
            r.push(i); accept(r.read());
            equal(r.readableLength, 0);
        }
    } else if (kind === 'bytes') {
        const r = new Readable();
        for (let i = 0; i < n; i++) r.push(Buffer.from([i % 251]));
        equal(r.readableLength, n);
        const bytes = r.read(); equal(bytes.length, n);
        for (let i = 0; i < n; i++) { equal(bytes[i], i % 251); checksum += bytes[i]; count++; }
        equal(r.readableLength, 0);
    } else if (kind === 'flow') {
        const r = new Readable({ objectMode: true });
        for (let i = 0; i < n; i++) r.push(i);
        r.push(null); r.on('data', accept);
        await new Promise(resolve => r.on('end', resolve));
    } else if (kind === 'writes') {
        let release;
        const w = new Writable({ objectMode: true, write(value, encoding, done) {
            accept(value); if (value === 0) release = done; else done();
        } });
        for (let i = 0; i < n; i++) w.write(i, error => { check(!error, 'write error'); equal(callbacks, i); callbacks++; });
        const finished = new Promise(resolve => w.end(resolve)); release(); await finished;
        equal(callbacks, n);
    } else if (kind === 'async') {
        const r = new Readable({ objectMode: true });
        for (let i = 0; i < n; i++) r.push(i);
        r.push(null);
        for await (const value of r) accept(value);
    } else { throw new Error('unknown case'); }
    equal(count, n);
    console.log(kind + ':' + n + ':' + count + ':' + checksum + ':' + callbacks);
}
"#;

#[test]
#[ignore = "release A/B benchmark; no timing assertions"]
fn node_stream_queue_scaling_benchmark() {
    // Exactly one repetition per case, identical fixtures in both preserved executables.
    // Parent measurement alternates entire executables ten times once other builds settle.
    for kind in ["objects", "bytes", "flow", "writes", "async", "trickle"] {
        for n in [1_u64, 16, 100, 1000, 10000] {
            let (output, elapsed) = execute(
                &format!("bench_{kind}_{n}"),
                BENCHMARK,
                &format!("runCase('{kind}', {n})"),
            );
            let checksum: u64 = (0..n)
                .map(|i| if kind == "bytes" { i % 251 } else { i })
                .sum();
            let callbacks = if kind == "writes" { n } else { 0 };
            assert_eq!(output, format!("{kind}:{n}:{n}:{checksum}:{callbacks}"));
            println!("verified {output} elapsed_us={}", elapsed.as_micros());
        }
    }
}
