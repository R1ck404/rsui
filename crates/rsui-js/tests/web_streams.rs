//! The web's streams. Most of these are a program whose output is what Node 24 prints for it, found by running the
//! same program in Node.

use std::cell::RefCell;
use std::rc::Rc;

use rsui_js::Engine;

fn run(name: &str, source: &str) -> String {
    let dir =
        std::env::temp_dir().join(format!("rsui_js_web_streams_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.js");
    std::fs::write(&file, source).unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine
        .run_file(&file)
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(result, Ok(()));
    let shown = output.borrow().replace("\r\n", "\n");
    shown.trim_end().to_owned()
}

/// What a stream does, and when, from being made to being read to its end.
#[test]
fn a_stream_pulls_and_reads_in_the_order_the_standard_says() {
    assert_eq!(
        run(
            "order",
            r##"
            const log = [];
            const s = new ReadableStream({
                start(c) { log.push('start'); c.enqueue('a'); },
                pull(c) { log.push('pull ' + c.desiredSize); if (log.length > 6) c.close(); else c.enqueue(String(log.length)); },
                cancel() { log.push('cancel'); },
            }, { highWaterMark: 2 });
            log.push('made');
            const r = s.getReader();
            Promise.resolve().then(() => log.push('tick 1')).then(() => log.push('tick 2')).then(() => log.push('tick 3'));
            r.read().then((x) => log.push('read ' + JSON.stringify(x)));
            r.read().then((x) => log.push('read ' + JSON.stringify(x)));
            setTimeout(async () => {
                while (true) {
                    const x = await r.read();
                    log.push('drain ' + JSON.stringify(x));
                    if (x.done) break;
                }
                r.closed.then(() => log.push('closed'));
                await r.closed;
                r.releaseLock();
                log.push('locked ' + s.locked);
                console.log(log.join('\n'));
            }, 10);
            "##
        ),
        "start\n\
         made\n\
         pull 2\n\
         tick 1\n\
         read {\"value\":\"a\",\"done\":false}\n\
         read {\"value\":\"3\",\"done\":false}\n\
         pull 2\n\
         tick 2\n\
         tick 3\n\
         drain {\"done\":true}\n\
         closed\n\
         locked false"
    );
}

/// Backpressure, on both kinds of stream.
#[test]
fn writing_waits_for_room_and_a_transform_holds_back_until_it_is_read() {
    assert_eq!(
        run(
            "back",
            r##"
            (async () => {
                const seen = [];
                const ws = new WritableStream({
                    start() { seen.push('start'); },
                    write(chunk) { seen.push('write ' + chunk); return new Promise((r) => setTimeout(r, 5)); },
                    close() { seen.push('close'); },
                }, { highWaterMark: 3, size: (chunk) => chunk.length });
                const w = ws.getWriter();
                seen.push('desired ' + w.desiredSize);
                const p1 = w.write('ab');
                seen.push('desired ' + w.desiredSize);
                const p2 = w.write('cd');
                seen.push('desired ' + w.desiredSize);
                w.ready.then(() => seen.push('ready, desired ' + w.desiredSize));
                await p1;
                seen.push('p1 done');
                await p2;
                seen.push('p2 done');
                await w.close();
                seen.push('closed, desired ' + w.desiredSize);
                // A transform holds back until its readable side is read.
                const t = new TransformStream({ transform(chunk, c) { seen.push('transform ' + chunk); c.enqueue(chunk * 2); } }, undefined, { highWaterMark: 1 });
                const tw = t.writable.getWriter();
                tw.write(1);
                tw.write(2);
                tw.write(3);
                await new Promise((r) => setTimeout(r, 5));
                seen.push('after writes, desired ' + tw.desiredSize);
                const tr = t.readable.getReader();
                seen.push('got ' + (await tr.read()).value);
                await new Promise((r) => setTimeout(r, 5));
                seen.push('got ' + (await tr.read()).value + ' ' + (await tr.read()).value);
                console.log(seen.join('\n'));
            })();
            "##
        ),
        "start\n\
         desired 3\n\
         desired 1\n\
         desired -1\n\
         write ab\n\
         write cd\n\
         p1 done\n\
         ready, desired 1\n\
         p2 done\n\
         close\n\
         closed, desired 0\n\
         transform 1\n\
         after writes, desired -1\n\
         transform 2\n\
         got 2\n\
         transform 3\n\
         got 4 6"
    );
}

/// Teeing, for streams of anything and streams of bytes.
#[test]
fn a_stream_in_two_halves_gives_each_everything_and_cancels_once_both_are_cancelled() {
    assert_eq!(
        run(
            "tee",
            r##"
            (async () => {
                const out = [];
                let pulls = 0;
                const [a, b] = new ReadableStream({
                    pull(c) { pulls += 1; c.enqueue(pulls); if (pulls === 3) c.close(); },
                    cancel(reason) { out.push('source cancelled ' + JSON.stringify(reason)); },
                }).tee();
                for await (const x of a) out.push('a ' + x);
                out.push('pulls ' + pulls);
                for await (const x of b) out.push('b ' + x);
                const [c, d] = new ReadableStream({ pull(cc) { cc.enqueue('x'); }, cancel(reason) { out.push('cancelled with ' + JSON.stringify(reason)); } }).tee();
                const first = c.cancel('one').then(() => out.push('one cancelled'));
                await d.cancel('two');
                await first;
                const [e, f] = new ReadableStream({ start(cc) { cc.error(new Error('broken')); } }).tee();
                await e.getReader().read().catch((err) => out.push('e ' + err.message));
                await f.getReader().closed.catch((err) => out.push('f ' + err.message));
                const bytes = new ReadableStream({ type: 'bytes', start(cc) { cc.enqueue(new Uint8Array([1, 2, 3])); cc.close(); } });
                const [g, h] = bytes.tee();
                const gv = (await g.getReader().read()).value;
                const hr = h.getReader({ mode: 'byob' });
                const hv = (await hr.read(new Uint8Array(2))).value;
                out.push('bytes ' + [...gv] + ' / ' + [...hv]);
                console.log(out.join('\n'));
            })();
            "##
        ),
        "a 1\n\
         a 2\n\
         a 3\n\
         pulls 3\n\
         b 1\n\
         b 2\n\
         b 3\n\
         cancelled with [\"one\",\"two\"]\n\
         one cancelled\n\
         e broken\n\
         f broken\n\
         bytes 1,2,3 / 1,2"
    );
}

/// Piping, with what goes wrong at either end.
#[test]
fn piping_passes_on_errors_closing_and_aborting_both_ways() {
    assert_eq!(
        run(
            "pipe",
            r##"
            (async () => {
                const out = [];
                const written = [];
                const source = () => new ReadableStream({ start(c) { c.enqueue(1); c.enqueue(2); c.close(); } });
                await source().pipeTo(new WritableStream({ write(x) { written.push(x); }, close() { out.push('dest closed'); } }));
                out.push('piped ' + written);
                await new ReadableStream({ start(c) { c.enqueue(1); }, pull(c) { c.error(new Error('source broke')); } })
                    .pipeTo(new WritableStream({ abort(reason) { out.push('dest aborted: ' + reason.message); } }))
                    .catch((e) => out.push('pipeTo: ' + e.message));
                await new ReadableStream({ pull(c) { c.enqueue('x'); }, cancel(reason) { out.push('source cancelled: ' + reason.message); } })
                    .pipeTo(new WritableStream({ write() { throw new Error('dest broke'); } }))
                    .catch((e) => out.push('pipeTo: ' + e.message));
                await new ReadableStream({ start(c) { c.error(new Error('kept open')); } })
                    .pipeTo(new WritableStream({ abort() { out.push('should not abort'); } }), { preventAbort: true })
                    .catch((e) => out.push('preventAbort: ' + e.message));
                const controller = new AbortController();
                const piping = new ReadableStream({
                    pull(c) { c.enqueue('tick'); },
                    cancel() { out.push('cancelled by signal'); },
                }).pipeTo(new WritableStream({ write(x) { out.push('wrote ' + x); controller.abort(); }, abort(reason) { out.push('aborted by signal: ' + reason.name); } }), { signal: controller.signal });
                await piping.catch((e) => out.push('signal: ' + e.name));
                const t = source().pipeThrough(new TransformStream({ transform(x, c) { c.enqueue(x * 10); } }));
                for await (const x of t) out.push('through ' + x);
                const locked = source();
                locked.getReader();
                try { locked.pipeThrough(new TransformStream()); } catch (e) { out.push(e.constructor.name); }
                await locked.pipeTo(new WritableStream()).catch((e) => out.push('locked pipeTo ' + e.constructor.name));
                console.log(out.join('\n'));
            })();
            "##
        ),
        "dest closed\n\
         piped 1,2\n\
         dest aborted: source broke\n\
         pipeTo: source broke\n\
         source cancelled: dest broke\n\
         pipeTo: dest broke\n\
         preventAbort: kept open\n\
         wrote tick\n\
         aborted by signal: AbortError\n\
         cancelled by signal\n\
         signal: AbortError\n\
         through 10\n\
         through 20\n\
         TypeError\n\
         locked pipeTo TypeError"
    );
}

/// Async iteration and `ReadableStream.from`.
#[test]
fn reading_with_for_await_can_stop_partway_and_cancels_the_stream_unless_told_not_to() {
    assert_eq!(
        run(
            "iter",
            r##"
            (async () => {
                const out = [];
                const make = (name) => new ReadableStream({
                    pull(c) { c.enqueue(name); },
                    cancel(reason) { out.push(name + ' cancelled ' + reason); },
                });
                const a = make('a');
                for await (const x of a) { out.push('got ' + x); break; }
                out.push('a locked ' + a.locked);
                const b = make('b');
                for await (const x of b.values({ preventCancel: true })) { out.push('got ' + x); break; }
                out.push('b locked ' + b.locked + ', still reads ' + (await b.getReader().read()).value);
                const c = make('c');
                const it = c[Symbol.asyncIterator]();
                const [n1, n2, r, n3] = await Promise.all([it.next(), it.next(), it.return('why'), it.next()]);
                out.push(JSON.stringify([n1, n2, r, n3]));
                const d = new ReadableStream({ start(cc) { cc.enqueue(1); cc.error(new Error('mid')); } });
                try { for await (const x of d) out.push('d ' + x); } catch (e) { out.push('d threw ' + e.message); }
                out.push(Object.prototype.toString.call(it), typeof Object.getPrototypeOf(Object.getPrototypeOf(it))[Symbol.asyncIterator]);
                for await (const x of ReadableStream.from((function* () { yield 1; yield Promise.resolve(2); })())) out.push('from ' + x);
                const gen = (async function* () { try { yield 'g'; yield 'never'; } finally { out.push('generator finished'); } })();
                for await (const x of ReadableStream.from(gen)) { out.push('from ' + x); break; }
                console.log(out.join('\n'));
            })();
            "##
        ),
        "got a\n\
         a cancelled undefined\n\
         a locked false\n\
         got b\n\
         b locked false, still reads b\n\
         c cancelled why\n\
         [{\"value\":\"c\",\"done\":false},{\"value\":\"c\",\"done\":false},{\"done\":true,\"value\":\"why\"},{\"done\":true}]\n\
         d threw mid\n\
         [object Object]\n\
         function\n\
         from 1\n\
         from 2\n\
         from g\n\
         generator finished"
    );
}

/// Text and compression streams.
#[test]
fn text_and_compression_streams_split_nothing_that_belongs_together() {
    assert_eq!(
        run(
            "text",
            r##"
            (async () => {
                const out = [];
                const bytes = new TextEncoder().encode('añ€😀!');
                const parts = [];
                for (let i = 0; i < bytes.length; i += 1) parts.push(bytes.slice(i, i + 1));
                const text = [];
                for await (const t of ReadableStream.from(parts).pipeThrough(new TextDecoderStream())) text.push(t);
                out.push(JSON.stringify(text));
                const pieces = [];
                for await (const b of ReadableStream.from(['a', '\ud83d', '\ude00', 'z']).pipeThrough(new TextEncoderStream())) pieces.push([...b].join(','));
                out.push(pieces.join(' | '));
                const tail = [];
                for await (const t of ReadableStream.from([new Uint8Array([0x61, 0xe2, 0x82])]).pipeThrough(new TextDecoderStream())) tail.push(t);
                out.push(JSON.stringify(tail));
                const fatal = ReadableStream.from([new Uint8Array([0xff])]).pipeThrough(new TextDecoderStream('utf-8', { fatal: true }));
                await fatal.getReader().read().catch((e) => out.push('fatal ' + e.constructor.name));
                const d = new TextDecoderStream();
                out.push(d.encoding + ' ' + d.fatal + ' ' + d.ignoreBOM + ' ' + new TextEncoderStream().encoding);
                const bom = [];
                for await (const t of ReadableStream.from([new Uint8Array([0xef, 0xbb, 0xbf, 0x41]), new Uint8Array([0xef, 0xbb, 0xbf, 0x42])]).pipeThrough(new TextDecoderStream())) bom.push(t);
                out.push(JSON.stringify(bom));
                for (const format of ['gzip', 'deflate', 'deflate-raw']) {
                    const input = new TextEncoder().encode('squeeze me '.repeat(50));
                    const round = ReadableStream.from([input]).pipeThrough(new CompressionStream(format)).pipeThrough(new DecompressionStream(format));
                    let got = '';
                    for await (const c of round.pipeThrough(new TextDecoderStream())) got += c;
                    out.push(format + ' ' + (got === 'squeeze me '.repeat(50)));
                }
                try { new CompressionStream('nope'); } catch (e) { out.push(e.constructor.name); }
                console.log(out.join('\n'));
            })();
            "##
        ),
        "[\"a\",\"\u{f1}\",\"\u{20ac}\",\"\u{1f600}\",\"!\"]\n\
         97 | 240,159,152,128 | 122\n\
         [\"a\",\"\u{fffd}\"]\n\
         fatal TypeError\n\
         utf-8 false false utf-8\n\
         [\"A\",\"\u{feff}B\"]\n\
         gzip true\n\
         deflate true\n\
         deflate-raw true\n\
         TypeError"
    );
}

/// Byte streams, their own readers and their requests.
#[test]
fn a_byte_stream_fills_what_it_is_given() {
    assert_eq!(
        run(
            "byob",
            r##"
            (async () => {
                const out = [];
                let n = 0;
                const s = new ReadableStream({
                    type: 'bytes',
                    pull(c) {
                        n += 1;
                        const req = c.byobRequest;
                        if (req) {
                            out.push('byob view ' + req.view.byteLength);
                            req.view[0] = n;
                            req.respond(1);
                        } else {
                            c.enqueue(new Uint8Array([n, n]));
                        }
                        if (n === 3) c.close();
                    },
                });
                const r = s.getReader({ mode: 'byob' });
                const buf = new Uint8Array(8);
                const first = await r.read(buf);
                out.push('first ' + [...first.value] + ' detached ' + (buf.byteLength === 0));
                const second = await r.read(new Uint16Array(2), { min: 1 }).catch((e) => e);
                out.push('second ' + (second.value ? second.value.constructor.name + ' ' + second.value.length : second));
                r.releaseLock();
                const d = s.getReader();
                out.push('default ' + JSON.stringify(await d.read()));
                const auto = new ReadableStream({
                    type: 'bytes',
                    autoAllocateChunkSize: 4,
                    pull(c) { out.push('auto ' + c.byobRequest.view.byteLength); c.byobRequest.view.set([9, 9]); c.byobRequest.respond(2); c.close(); },
                });
                const ar = await auto.getReader().read();
                out.push('auto read ' + [...ar.value]);
                try { new ReadableStream({ type: 'nope' }); } catch (e) { out.push(e.constructor.name); }
                try { new ReadableStream({}).getReader({ mode: 'byob' }); } catch (e) { out.push(e.constructor.name); }
                try { new ReadableStream({ type: 'bytes' }, { size() { return 1; } }); } catch (e) { out.push(e.constructor.name); }
                out.push(new CountQueuingStrategy({ highWaterMark: 5 }).size() + ' ' + new ByteLengthQueuingStrategy({ highWaterMark: 5 }).size(new Uint8Array(3)));
                console.log(out.join('\n'));
            })();
            "##
        ),
        "byob view 8\n\
         first 1 detached true\n\
         byob view 4\n\
         byob view 3\n\
         second Uint16Array 1\n\
         default {\"done\":true}\n\
         auto 4\n\
         auto read 9,9\n\
         TypeError\n\
         TypeError\n\
         RangeError\n\
         1 3"
    );
}

/// Node's own ways between its streams and the web's, `stream/web`, and `Blob.stream()`.
#[test]
fn node_streams_and_web_streams_turn_into_each_other() {
    assert_eq!(
        run(
            "interop",
            r##"
            const { Readable, Writable, Duplex, PassThrough } = require('stream');
            const web = require('stream/web');
            (async () => {
                const out = [];
                out.push('stream/web ' + (web.ReadableStream === ReadableStream) + ' ' + Object.keys(web).length);
                let text = '';
                for await (const part of new Blob(['blob ', 'text']).stream()) text += new TextDecoder().decode(part);
                out.push(text);
                const fromNode = Readable.toWeb(Readable.from([Buffer.from('x'), Buffer.from('y')], { objectMode: false }));
                const got = [];
                for await (const part of fromNode) got.push(part.constructor.name + ' ' + String.fromCharCode(...part));
                out.push('toWeb ' + got.join(', '));
                const objects = Readable.toWeb(Readable.from([{ a: 1 }, { a: 2 }]));
                for await (const part of objects) out.push('object ' + part.a);
                const back = Readable.fromWeb(new ReadableStream({ start(c) { c.enqueue(new Uint8Array([65, 66])); c.close(); } }));
                for await (const part of back) out.push('fromWeb ' + Buffer.isBuffer(part) + ' ' + part);
                const failing = Readable.fromWeb(new ReadableStream({ start(c) { c.error(new Error('web broke')); } }));
                await new Promise((resolve) => failing.on('error', (error) => { out.push('fromWeb error ' + error.message); resolve(); }).resume());
                const sink = [];
                const toSink = Writable.toWeb(new Writable({ write(chunk, encoding, done) { sink.push(String(chunk)); done(); } }));
                const writer = toSink.getWriter();
                await writer.write(new Uint8Array([67]));
                await writer.close();
                out.push('Writable.toWeb ' + sink);
                const written = [];
                const toWeb = Writable.fromWeb(new WritableStream({ write(chunk) { written.push(chunk.constructor.name + ' ' + String.fromCharCode(...chunk)); } }));
                toWeb.write('Z');
                await new Promise((resolve) => toWeb.end(resolve));
                out.push('Writable.fromWeb ' + written);
                const pair = Duplex.toWeb(new PassThrough());
                const pairWriter = pair.writable.getWriter();
                pairWriter.write(new TextEncoder().encode('through'));
                pairWriter.close();
                for await (const part of pair.readable) out.push('Duplex.toWeb ' + new TextDecoder().decode(part));
                const view = new Uint8Array([1, 2, 3]);
                const moved = structuredClone(view, { transfer: [view.buffer] });
                out.push('clone ' + moved.constructor.name + ' ' + [...moved] + ' ' + view.byteLength + ' ' + view.buffer.detached);
                const shared = new Uint8Array(new ArrayBuffer(4));
                const cloned = structuredClone({ a: shared, b: new DataView(shared.buffer, 1, 2) });
                out.push('views ' + (cloned.b.buffer === cloned.a.buffer) + ' ' + cloned.b.constructor.name + ' ' + cloned.b.byteOffset + ' ' + cloned.b.byteLength);
                const response = new Response(Readable.toWeb(Readable.from(['made ', 'in node'].map((part) => Buffer.from(part)))));
                out.push('response ' + await response.text());
                console.log(out.join('\n'));
            })();
            "##
        ),
        "stream/web true 17\n\
         blob text\n\
         toWeb Uint8Array x, Uint8Array y\n\
         object 1\n\
         object 2\n\
         fromWeb true AB\n\
         fromWeb error web broke\n\
         Writable.toWeb C\n\
         Writable.fromWeb Buffer Z\n\
         Duplex.toWeb through\n\
         clone Uint8Array 1,2,3 0 true\n\
         views true DataView 1 2\n\
         response made in node"
    );
}

/// `Response`, `Request` and `Headers`, with bodies that are bytes or streams.
#[test]
fn a_response_or_request_body_is_read_once_however_it_is_read() {
    assert_eq!(
        run(
            "body",
            r##"
            (async () => {
                const out = [];
                const plain = new Response('hello');
                out.push(plain.status + ' ' + plain.ok + ' ' + plain.headers.get('content-type') + ' ' + plain.bodyUsed);
                out.push(await plain.text() + ' ' + plain.bodyUsed);
                await plain.text().catch((error) => out.push('again ' + error.constructor.name));
                const streamed = new Response(new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode('{"a":')); c.enqueue(new TextEncoder().encode('1}')); c.close(); } }));
                const copy = streamed.clone();
                out.push(JSON.stringify(await streamed.json()) + ' ' + new TextDecoder().decode(await copy.arrayBuffer()));
                const body = new Response(new Uint8Array([1, 2, 3])).body;
                const reader = body.getReader();
                const first = await reader.read();
                out.push('body ' + first.value.constructor.name + ' ' + [...first.value] + ' ' + JSON.stringify(await reader.read()));
                const read = new Response('x');
                read.body.getReader();
                out.push('locked used ' + read.bodyUsed);
                await read.text().catch((error) => out.push('locked ' + error.constructor.name));
                out.push('empty ' + (new Response().body === null) + ' ' + JSON.stringify(await new Response().text()));
                const json = Response.json({ ok: true }, { status: 201 });
                out.push(json.status + ' ' + json.headers.get('content-type') + ' ' + await json.text());
                out.push(Response.redirect('http://example.com/a', 301).headers.get('location') + ' ' + Response.error().type + ' ' + Response.error().status);
                try { new Response('x', { status: 204 }); } catch (error) { out.push('204 ' + error.constructor.name); }
                try { new Response('x', { status: 99 }); } catch (error) { out.push('99 ' + error.constructor.name); }
                const request = new Request('http://example.com/path?q=1', { method: 'post', body: 'sent', headers: { 'X-One': '1' } });
                out.push(request.method + ' ' + request.url + ' ' + request.headers.get('x-one') + ' ' + request.headers.get('content-type'));
                out.push(await request.clone().text() + ' ' + await request.text() + ' ' + request.bodyUsed);
                try { new Request('http://example.com/', { method: 'GET', body: 'x' }); } catch (error) { out.push('GET body ' + error.constructor.name); }
                try { new Request('http://example.com/', { method: 'POST', body: new ReadableStream() }); } catch (error) { out.push('no duplex ' + error.constructor.name); }
                const moved = new Request(new Request('http://example.com/', { method: 'PUT', body: 'moved' }));
                out.push(moved.method + ' ' + await moved.text());
                const headers = new Headers([['b', '2'], ['a', '1'], ['Set-Cookie', 'x=1'], ['set-cookie', 'y=2']]);
                out.push([...headers].map(([name, value]) => name + '=' + value).join('&') + ' ' + headers.getSetCookie().length);
                const blob = await new Response('as a blob', { headers: { 'content-type': 'text/plain' } }).blob();
                out.push(blob.size + ' ' + blob.type + ' ' + await blob.text());
                const bytes = await new Response('ab').bytes();
                out.push(bytes.constructor.name + ' ' + bytes.length);
                const form = new FormData();
                form.append('a', '1');
                form.append('file', new Blob(['data'], { type: 'text/plain' }), 'f.txt');
                form.append('a', 'again');
                form.set('b', 'two');
                out.push('form ' + form.getAll('a') + ' ' + form.has('file') + ' ' + form.get('file').name);
                const sent = new Response(form);
                out.push('form type ' + sent.headers.get('content-type').startsWith('multipart/form-data; boundary='));
                const back = await sent.formData();
                out.push('form back ' + [...back].map(([key, value]) => key + '=' + (typeof value === 'string' ? value : value.name + ':' + value.type + ':' + value.size)).join('&'));
                const encoded = await new Response(new URLSearchParams('x=1&y=two words')).formData();
                out.push('urlencoded ' + [...encoded].join('|'));
                console.log(out.join('\n'));
            })();
            "##
        ),
        "200 true text/plain;charset=UTF-8 false\n\
         hello true\n\
         again TypeError\n\
         {\"a\":1} {\"a\":1}\n\
         body Uint8Array 1,2,3 {\"done\":true}\n\
         locked used false\n\
         locked TypeError\n\
         empty true \"\"\n\
         201 application/json {\"ok\":true}\n\
         http://example.com/a error 0\n\
         204 TypeError\n\
         99 RangeError\n\
         POST http://example.com/path?q=1 1 text/plain;charset=UTF-8\n\
         sent sent true\n\
         GET body TypeError\n\
         no duplex TypeError\n\
         PUT moved\n\
         a=1&b=2&set-cookie=x=1&set-cookie=y=2 2\n\
         9 text/plain as a blob\n\
         Uint8Array 2\n\
         form 1,again true f.txt\n\
         form type true\n\
         form back a=1&file=f.txt:text/plain:4&a=again&b=two\n\
         urlencoded x,1|y,two words"
    );
}

/// The streams are only compiled for a program that uses them, and reading a body whole doesn't need them.
#[test]
fn streams_are_only_compiled_when_something_touches_them() {
    let compiled = |source: &str| {
        let mut engine = Engine::new();
        engine.set_printer(|_| {});
        engine.eval(source, "main.js").unwrap();
        engine.run_until_idle().unwrap();
        engine
            .loaded_builtins()
            .contains(&"internal/misc/web_streams".to_owned())
    };
    assert!(!compiled("1 + 1"));
    assert!(!compiled(
        "new Response('hi').text().then((text) => text.length)"
    ));
    assert!(compiled("new Response('hi').body.getReader()"));
    assert!(compiled("typeof ReadableStream"));
}
