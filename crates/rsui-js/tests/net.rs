//! Sockets, web sockets and workers, each talking to the other end of itself on this machine. Everything here listens only on
//! `127.0.0.1`, as listening anywhere else can bring up a question from the firewall.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use rsui_js::Engine;

/// How long a test here may take, however busy the machine is, before it's taken as stuck. Each takes well under a
/// second on its own.
const STUCK_AFTER: Duration = Duration::from_secs(120);

/// Runs a test on a thread of its own and fails it if it hasn't finished in [`STUCK_AFTER`], rather than waiting for ever
/// on something that's never coming. The stuck thread is left behind, and goes when the tests do.
fn in_time<T: Send + 'static>(name: &str, run: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _ = done.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)));
        })
        .expect("a thread starts");
    match finished.recv_timeout(STUCK_AFTER) {
        Ok(Ok(value)) => value,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => panic!("{name} was still going after {STUCK_AFTER:?}"),
    }
}

fn run_in(name: &str, files: &[(&str, &str)]) -> String {
    let owned: Vec<(String, String)> =
        files.iter().map(|(file, source)| ((*file).to_owned(), (*source).to_owned())).collect();
    let label = name.to_owned();
    in_time(name, move || {
        let files: Vec<(&str, &str)> = owned.iter().map(|(file, source)| (file.as_str(), source.as_str())).collect();
        run_here(&label, &files)
    })
}

fn run_here(name: &str, files: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("rsui_js_net_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (file, source) in files {
        std::fs::write(dir.join(file), source).unwrap();
    }
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine
        .run_file(&dir.join(files[0].0))
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(result, Ok(()));
    let shown = output.borrow().replace("\r\n", "\n");
    shown.trim_end().to_owned()
}

fn run(name: &str, source: &str) -> String {
    run_in(name, &[("main.js", source)])
}

#[test]
fn a_server_and_a_client_talk_over_tcp() {
    assert_eq!(
        run(
            "tcp",
            r#"
            const net = require('net');
            const server = net.createServer((socket) => {
                console.log('server sees', socket.remoteAddress);
                socket.setEncoding('utf8');
                socket.on('data', (text) => socket.write(text.toUpperCase()));
                socket.on('end', () => {
                    console.log('server end');
                    socket.end();
                });
            });
            server.listen(0, '127.0.0.1', () => {
                const { address, port, family } = server.address();
                console.log(address, port > 0, family, server.listening);
                const client = net.connect(port, '127.0.0.1');
                console.log(client.connecting);
                client.write('written before it was open ');
                client.on('connect', () => {
                    console.log('connected', client.remoteAddress, client.remotePort === port, client.readyState);
                    client.write(Buffer.from('and after'));
                });
                let heard = '';
                client.on('data', (chunk) => {
                    heard += chunk;
                    if (heard.endsWith('AFTER')) client.end();
                });
                client.on('end', () => console.log('client end', heard));
                client.on('close', (failed) => {
                    console.log('client closed', failed, client.readyState);
                    server.close((error) => console.log('server closed', error, server.listening));
                });
            });
            "#
        ),
        "127.0.0.1 true IPv4 true\n\
         true\n\
         connected 127.0.0.1 true open\n\
         server sees 127.0.0.1\n\
         server end\n\
         client end WRITTEN BEFORE IT WAS OPEN AND AFTER\n\
         client closed false closed\n\
         server closed undefined false"
    );
}

#[test]
fn connecting_to_nothing_says_what_node_says() {
    assert_eq!(
        run(
            "refused",
            r#"
            const net = require('net');
            const server = net.createServer();
            server.listen(0, '127.0.0.1', () => {
                const { port } = server.address();
                server.close(() => {
                    const client = net.connect({ port, host: '127.0.0.1' });
                    client.on('error', (error) => console.log(error.code, error.message.startsWith('connect ECONNREFUSED 127.0.0.1')));
                    client.on('close', (failed) => console.log('closed', failed));
                });
            });
            "#
        ),
        "ECONNREFUSED true\nclosed true"
    );
}

#[test]
fn listening_where_something_already_is_says_what_node_says() {
    assert_eq!(
        run(
            "in-use",
            r#"
            const net = require('net');
            const first = net.createServer();
            first.listen(0, '127.0.0.1', () => {
                const second = net.createServer();
                second.on('error', (error) => {
                    console.log(error.code);
                    first.close();
                });
                second.listen(first.address().port, '127.0.0.1');
            });
            "#
        ),
        "EADDRINUSE"
    );
}

#[test]
// Windows' own: its named pipes, and `findstr`.
#[cfg(windows)]
fn a_named_pipe_carries_both_ways_at_once() {
    let name = format!(r"\\.\pipe\rsui-test-{}", std::process::id());
    let source = format!(
        r#"
        const net = require('net');
        const path = {name:?};
        const server = net.createServer((socket) => {{
            socket.on('data', (chunk) => socket.write('echo ' + chunk));
            socket.on('close', () => server.close(() => console.log('server closed')));
        }});
        server.listen(path, () => {{
            console.log(server.address() === path);
            const second = net.createServer();
            second.on('error', (error) => console.log('second', error.code));
            second.listen(path);
            const client = net.connect(path, () => {{
                // It's reading the whole time, which mustn't hold this up.
                client.write('one');
            }});
            client.setEncoding('utf8');
            let count = 0;
            client.on('data', (text) => {{
                console.log(text);
                count += 1;
                if (count === 1) client.write('two');
                else client.destroy();
            }});
            client.on('close', () => console.log('client closed'));
        }});
        "#
    );
    let said = run("pipe", &source);
    let mut lines: Vec<&str> = said.lines().collect();
    // Which of these comes first is up to how the threads go.
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "client closed",
            "echo one",
            "echo two",
            "second EADDRINUSE",
            "server closed",
            "true"
        ]
    );
}

#[test]
// Windows' own: its named pipes, and `findstr`.
#[cfg(windows)]
fn a_named_pipe_can_be_written_the_other_way() {
    let name = format!(r"\\?\pipe\rsui-test-other-{}", std::process::id());
    let source = format!(
        r#"
        const net = require('net');
        const path = {name:?};
        const server = net.createServer((socket) => socket.once('data', (data) => socket.end('got ' + data)));
        server.listen(path, () => {{
            const client = net.createConnection(path);
            client.on('connect', () => client.write('it'));
            client.on('data', (text) => console.log(String(text)));
            client.on('close', () => server.close());
        }});
        "#
    );
    assert_eq!(run("pipe-other", &source), "got it");
}

#[test]
fn a_named_pipe_that_is_not_there_says_so() {
    assert_eq!(
        run(
            "no-pipe",
            r#"
            const client = require('net').connect('\\\\.\\pipe\\rsui-nothing-here-at-all');
            client.on('error', (error) => console.log(error.code));
            "#
        ),
        "ENOENT"
    );
}

#[test]
fn a_socket_times_out_when_nothing_is_happening() {
    assert_eq!(
        run(
            "timeout",
            r#"
            const net = require('net');
            const server = net.createServer();
            let client;
            server.listen(0, '127.0.0.1', () => {
                client = net.connect(server.address().port, '127.0.0.1');
            });
            // Only once the server has it: on a busy machine 20 ms can go by before then, and the server closed with nothing to
            // say was gone.
            server.on('connection', (socket) => {
                socket.on('close', () => console.log('gone'));
                client.setTimeout(20, () => {
                    console.log('timed out');
                    client.destroy();
                    server.close();
                });
            });
            "#
        ),
        "timed out\ngone"
    );
}

#[test]
fn names_are_looked_up_without_going_anywhere() {
    assert_eq!(
        run(
            "dns",
            r#"
            const dns = require('dns');
            dns.lookup('127.0.0.1', (error, address, family) => {
                console.log(error, address, family);
                dns.promises.lookup('127.0.0.1', { all: true }).then((all) => {
                    console.log(all.length, all[0].address);
                    console.log(require('net').isIP('::1'), require('net').isIP('nope'));
                });
            });
            "#
        ),
        "null 127.0.0.1 4\n1 127.0.0.1\n6 0"
    );
}

#[test]
fn udp_goes_from_one_socket_to_another() {
    let said = run(
        "udp",
        r#"
            const dgram = require('dgram');
            const hearing = dgram.createSocket('udp4');
            hearing.on('message', (message, from) => {
                console.log(String(message), from.address, from.size);
                hearing.close(() => console.log('closed'));
                sending.close();
            });
            const sending = dgram.createSocket('udp4');
            hearing.bind(0, '127.0.0.1', () => {
                const { port, address } = hearing.address();
                console.log(address);
                sending.send('hello', port, '127.0.0.1', (error, length) => console.log(error, length));
            });
            "#,
    );
    let mut lines: Vec<&str> = said.lines().collect();
    // What's sent can get there before this hears it's gone.
    lines.sort_unstable();
    assert_eq!(
        lines,
        ["127.0.0.1", "closed", "hello 127.0.0.1 5", "null 5"]
    );
}

#[test]
fn an_http_server_answers_fetch() {
    assert_eq!(
        run(
            "http-server",
            r#"
            const http = require('http');
            const server = http.createServer((request, response) => {
                let body = '';
                request.setEncoding('utf8');
                request.on('data', (chunk) => (body += chunk));
                request.on('end', () => {
                    if (request.url === '/stream') {
                        response.writeHead(201, { 'X-Made': 'yes' });
                        response.write('in ');
                        response.write('parts');
                        response.end();
                    } else {
                        response.setHeader('Content-Type', 'application/json');
                        response.end(JSON.stringify({ method: request.method, url: request.url, body, wanted: request.headers['x-wanted'] }));
                    }
                });
            });
            server.listen(0, '127.0.0.1', async () => {
                const base = 'http://127.0.0.1:' + server.address().port;
                const posted = await fetch(base + '/a?b=1', { method: 'POST', body: 'hello', headers: { 'X-Wanted': 'this' } });
                console.log(posted.status, posted.headers.get('content-type'), JSON.stringify(await posted.json()));
                const parts = await fetch(base + '/stream');
                console.log(parts.status, parts.headers.get('x-made'), await parts.text());
                server.close(() => console.log('closed'));
            });
            "#
        ),
        "200 application/json {\"method\":\"POST\",\"url\":\"/a?b=1\",\"body\":\"hello\",\"wanted\":\"this\"}\n\
         201 yes in parts\n\
         closed"
    );
}

/// A server that speaks web sockets, written with what's being tested, so that nothing else is needed.
const WEB_SOCKET_SERVER: &str = r#"
    const net = require('net');
    const crypto = require('crypto');
    function frame(opcode, payload) {
        const head = payload.length < 126 ? Buffer.from([0x80 | opcode, payload.length]) : Buffer.from([0x80 | opcode, 126, payload.length >> 8, payload.length & 255]);
        return Buffer.concat([head, payload]);
    }
    function serve(onReady) {
        const server = net.createServer((socket) => {
            let shaken = false;
            let held = Buffer.alloc(0);
            socket.on('error', () => {});
            socket.on('data', (chunk) => {
                held = Buffer.concat([held, chunk]);
                if (!shaken) {
                    const end = held.indexOf('\r\n\r\n');
                    if (end < 0) return;
                    const head = held.subarray(0, end).toString();
                    held = held.subarray(end + 4);
                    const key = /Sec-WebSocket-Key: (.*)/i.exec(head)[1].trim();
                    const accept = crypto.createHash('sha1').update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
                    const wanted = /Sec-WebSocket-Protocol: (.*)/i.exec(head);
                    socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n' +
                        (wanted ? 'Sec-WebSocket-Protocol: ' + wanted[1].trim().split(',')[0] + '\r\n' : '') + '\r\n');
                    shaken = true;
                }
                while (held.length >= 2) {
                    const opcode = held[0] & 15;
                    let length = held[1] & 127;
                    let at = 2;
                    if (length === 126) {
                        if (held.length < 4) return;
                        length = (held[2] << 8) | held[3];
                        at = 4;
                    }
                    if (held.length < at + 4 + length) return;
                    const mask = held.subarray(at, at + 4);
                    const payload = Buffer.from(held.subarray(at + 4, at + 4 + length));
                    for (let index = 0; index < length; index += 1) payload[index] ^= mask[index % 4];
                    held = held.subarray(at + 4 + length);
                    if (opcode === 8) {
                        socket.write(frame(8, payload));
                        socket.end();
                        return;
                    }
                    socket.write(frame(opcode, payload));
                }
            });
        });
        server.listen(0, '127.0.0.1', () => onReady(server, 'ws://127.0.0.1:' + server.address().port + '/path'));
        return server;
    }
"#;

#[test]
#[cfg_attr(not(windows), ignore = "WebSockets aren't on this system yet")]
fn a_web_socket_opens_and_echoes_text_and_bytes_and_closes() {
    let source = format!(
        r#"{WEB_SOCKET_SERVER}
        serve((server, url) => {{
            const socket = new WebSocket(url, ['chat', 'other']);
            console.log(socket.readyState === WebSocket.CONNECTING, socket.url === url);
            try {{ socket.send('too soon'); }} catch (error) {{ console.log(error.name); }}
            socket.binaryType = 'arraybuffer';
            socket.onopen = () => {{
                console.log('open', socket.readyState === WebSocket.OPEN, socket.protocol);
                socket.send('héllo');
                socket.send(new Uint8Array([1, 2, 3]));
                socket.send('x'.repeat(40000 > 60000 ? 0 : 300));
            }};
            let count = 0;
            socket.addEventListener('message', (event) => {{
                count += 1;
                if (typeof event.data === 'string') console.log('text', event.data.length, event.data.slice(0, 5));
                else console.log('bytes', [...new Uint8Array(event.data)].join());
                if (count === 3) socket.close(3001, 'done');
            }});
            socket.onerror = (event) => console.log('error', event.message);
            socket.onclose = (event) => {{
                console.log('close', event.code, event.reason, event.wasClean, socket.readyState === WebSocket.CLOSED);
                server.close();
            }};
        }});
        "#
    );
    assert_eq!(
        run("ws", &source),
        "true true\n\
         InvalidStateError\n\
         open true chat\n\
         text 5 héllo\n\
         bytes 1,2,3\n\
         text 300 xxxxx\n\
         close 3001 done true true"
    );
}

#[test]
fn a_web_socket_to_nothing_fails_and_closes() {
    assert_eq!(
        run(
            "ws-refused",
            r#"
            const server = require('net').createServer();
            server.listen(0, '127.0.0.1', () => {
                const url = 'ws://127.0.0.1:' + server.address().port;
                server.close(() => {
                    const socket = new WebSocket(url);
                    socket.onerror = () => console.log('error');
                    socket.onclose = (event) => console.log('close', event.code, event.wasClean);
                });
            });
            try { new WebSocket('http://nope'); } catch (error) { console.log(error.name); }
            "#
        ),
        "SyntaxError\nerror\nclose 1006 false"
    );
}

#[test]
fn a_worker_runs_on_its_own_and_hears_and_says_things() {
    assert_eq!(
        run_in(
            "worker",
            &[
                (
                    "main.js",
                    r#"
                    const { Worker, isMainThread, threadId } = require('worker_threads');
                    console.log(isMainThread, threadId);
                    const worker = new Worker('./inner.js', { workerData: { start: 5, list: [1, 2] } });
                    worker.on('online', () => console.log('online'));
                    const map = new Map([['key', new Date(1000)]]);
                    const shared = { name: 'shared' };
                    worker.postMessage({ map, set: new Set([1]), bytes: new Uint8Array([9, 8]), buffer: new ArrayBuffer(2), one: shared, two: shared });
                    worker.on('message', (message) => {
                        console.log(JSON.stringify(message));
                        if (message.done) worker.postMessage('stop');
                    });
                    worker.on('exit', (code) => console.log('exit', code));
                    try {
                        worker.postMessage(() => {});
                    } catch (error) {
                        console.log(error.message);
                    }
                    "#,
                ),
                (
                    "inner.js",
                    r#"
                    const { parentPort, workerData, isMainThread, threadId } = require('worker_threads');
                    parentPort.postMessage({ isMainThread, hasId: threadId > 0, workerData });
                    parentPort.on('message', (message) => {
                        if (message === 'stop') {
                            parentPort.close();
                            return;
                        }
                        const [key, date] = [...message.map][0];
                        parentPort.postMessage({
                            key,
                            time: date.getTime(),
                            isDate: date instanceof Date,
                            set: message.set.has(1),
                            bytes: message.bytes instanceof Uint8Array ? [...message.bytes] : 'lost',
                            buffer: message.buffer.byteLength,
                            sameThing: message.one === message.two,
                            done: true,
                        });
                    });
                    "#,
                ),
            ],
        ),
        "true 0\n\
         A function could not be cloned\n\
         online\n\
         {\"isMainThread\":false,\"hasId\":true,\"workerData\":{\"start\":5,\"list\":[1,2]}}\n\
         {\"key\":\"key\",\"time\":1000,\"isDate\":true,\"set\":true,\"bytes\":[9,8],\"buffer\":2,\"sameThing\":true,\"done\":true}\n\
         exit 0"
    );
}

#[test]
fn a_worker_with_nothing_left_to_do_ends_by_itself() {
    assert_eq!(
        run_in(
            "worker-ends",
            &[
                (
                    "main.js",
                    r#"
                    const { Worker } = require('worker_threads');
                    new Worker('./inner.js').on('message', (text) => console.log(text)).on('exit', (code) => console.log('exit', code));
                    "#,
                ),
                (
                    "inner.js",
                    r#"
                    require('worker_threads').parentPort.postMessage('from a timer later');
                    setTimeout(() => require('worker_threads').parentPort.postMessage('after a wait'), 5);
                    "#,
                ),
            ],
        ),
        "from a timer later\nafter a wait\nexit 0"
    );
}

#[test]
fn a_worker_that_fails_says_why_and_ends() {
    assert_eq!(
        run_in(
            "worker-fails",
            &[
                (
                    "main.js",
                    r#"
                    const { Worker } = require('worker_threads');
                    const worker = new Worker('./inner.js');
                    worker.on('error', (error) => console.log(error.message.includes('went wrong')));
                    worker.on('exit', (code) => console.log('exit', code));
                    new Worker('./not-there.js').on('error', () => console.log('no such file')).on('exit', (code) => console.log('exit', code));
                    "#,
                ),
                ("inner.js", "setTimeout(() => {}, 0); throw new Error('it went wrong');"),
            ],
        )
        .lines()
        .filter(|line| *line != "exit 1")
        .count(),
        2
    );
}

#[test]
fn the_web_worker_is_the_same_thing_as_the_web_writes_it() {
    assert_eq!(
        run_in(
            "web-worker",
            &[
                (
                    "main.js",
                    r#"
                    const worker = new Worker('inner.ts');
                    worker.onmessage = (event) => {
                        console.log(event.data);
                        worker.terminate();
                    };
                    worker.onerror = (event) => console.log('error', event.message);
                    worker.postMessage(20);
                    "#,
                ),
                (
                    "inner.ts",
                    r#"
                    const doubled = (value: number): number => value * 2;
                    self.onmessage = (event: any) => postMessage('doubled ' + doubled(event.data) + ' ' + (self === globalThis));
                    "#,
                ),
            ],
        ),
        "doubled 40 true"
    );
}

#[test]
fn stopping_a_worker_gives_a_promise_of_it_having_stopped() {
    assert_eq!(
        run_in(
            "worker-stop",
            &[
                (
                    "main.js",
                    r#"
                    const { Worker } = require('worker_threads');
                    const worker = new Worker('./inner.js');
                    worker.on('online', async () => console.log('stopped with', await worker.terminate()));
                    "#,
                ),
                ("inner.js", "setInterval(() => {}, 1000);"),
            ],
        ),
        "stopped with 0"
    );
}

#[test]
fn an_error_made_by_a_class_with_a_getter_for_its_name_does_not_run_that_early() {
    assert_eq!(
        run(
            "error-getter",
            r#"
            let ran = 0;
            class Named extends Error {
                #name;
                constructor(message, name) {
                    super(message);
                    this.#name = name;
                }
                get name() {
                    ran += 1;
                    return this.#name;
                }
            }
            const made = new Named('it broke', 'Special');
            console.log(ran, made.name, made.message, String(made));
            console.log(made.stack.split('\n')[0]);
            const dom = new DOMException('not allowed', 'SyntaxError');
            console.log(dom.name, dom.code, dom.message, dom instanceof Error);
            class Plain extends Error {}
            Plain.prototype.name = 'Plain';
            console.log(new Plain('words').stack.split('\n')[0]);
            "#
        ),
        "0 Special it broke Special: it broke\n\
         Named: it broke\n\
         SyntaxError 12 not allowed true\n\
         Plain: words"
    );
}

#[test]
// Windows' own: its named pipes, and `findstr`.
#[cfg(windows)]
fn stopping_a_pipe_server_right_as_it_is_about_to_wait_cannot_get_stuck() {
    let name = format!(r"\\.\pipe\rsui-test-stop-{}", std::process::id());
    let source = format!(
        r#"
        const net = require('net');
        const path = {name:?};
        // Stopping straight after each connection is what lands in the gap, some of the time.
        let left = 200;
        function round() {{
            if (left === 0) {{
                console.log('finished');
                return;
            }}
            left -= 1;
            const server = net.createServer((socket) => {{
                socket.destroy();
                server.close(round);
            }});
            server.listen(path, () => {{
                const client = net.connect(path);
                client.on('error', () => {{}});
                client.on('close', () => {{}});
                client.on('data', () => {{}});
            }});
        }}
        round();
        "#
    );
    assert_eq!(run("pipe-stop", &source), "finished");
}

#[test]
fn listening_once_for_data_starts_a_stream_flowing_as_listening_does() {
    assert_eq!(
        run(
            "once-flows",
            r#"
            const { Readable } = require('stream');
            const stream = new Readable({ read() {} });
            stream.once('data', (chunk) => console.log('first', String(chunk)));
            stream.on('end', () => console.log('ended'));
            stream.push('one');
            stream.push('two');
            stream.push(null);
            const EventEmitter = require('events');
            class Counting extends EventEmitter {
                on(name, listener) {
                    console.log('on', name);
                    return super.on(name, listener);
                }
            }
            const counting = new Counting();
            counting.once('thing', () => console.log('thing'));
            counting.emit('thing');
            counting.emit('thing');
            "#
        ),
        "on thing\nthing\nfirst one\nended"
    );
}

#[test]
fn many_connections_at_once_each_hear_their_own_answer() {
    assert_eq!(
        run(
            "many",
            r#"
            const net = require('net');
            const count = 200;
            const server = net.createServer((socket) => {
                socket.setEncoding('utf8');
                socket.on('data', (text) => socket.end('echo ' + text));
            });
            server.listen(0, '127.0.0.1', () => {
                const { port } = server.address();
                let right = 0;
                let left = count;
                for (let index = 0; index < count; index += 1) {
                    const client = net.connect(port, '127.0.0.1', () => client.write(String(index)));
                    let heard = '';
                    client.setEncoding('utf8');
                    client.on('data', (text) => (heard += text));
                    client.on('close', () => {
                        if (heard === 'echo ' + index) right += 1;
                        left -= 1;
                        if (left === 0) {
                            console.log(right);
                            server.close();
                        }
                    });
                }
            });
            "#
        ),
        "200"
    );
}

#[test]
fn a_lot_written_at_once_comes_back_whole() {
    assert_eq!(
        run(
            "a-lot",
            r#"
            const net = require('net');
            const size = 4 * 1024 * 1024 + 3;
            const sent = Buffer.alloc(size);
            for (let index = 0; index < size; index += 1) sent[index] = (index * 7) & 255;
            const server = net.createServer((socket) => {
                socket.on('data', (chunk) => socket.write(chunk));
                socket.on('end', () => socket.end());
            });
            server.listen(0, '127.0.0.1', () => {
                const client = net.connect(server.address().port, '127.0.0.1', () => client.end(sent));
                const parts = [];
                client.on('data', (chunk) => parts.push(chunk));
                client.on('end', () => {
                    const got = Buffer.concat(parts);
                    console.log(got.length === size, got.equals(sent), client.bytesWritten === size);
                    server.close();
                });
            });
            "#
        ),
        "true true true"
    );
}

#[test]
fn udp_sent_to_nowhere_in_particular_goes_to_this_machine() {
    assert_eq!(
        run(
            "udp-local",
            r#"
            const dgram = require('dgram');
            const hearing = dgram.createSocket('udp4');
            const sending = dgram.createSocket('udp4');
            hearing.on('message', (message, from) => {
                console.log(String(message), from.address);
                hearing.close();
                sending.close();
            });
            hearing.bind(0, '127.0.0.1', () => sending.send('to localhost', hearing.address().port));
            "#
        ),
        "to localhost 127.0.0.1"
    );
}

#[test]
// Windows' own: its named pipes, and `findstr`.
#[cfg(windows)]
fn another_program_is_heard_and_told_things_and_finishes() {
    assert_eq!(
        run(
            "child",
            r#"
            const { spawn } = require('child_process');
            const child = spawn('cmd.exe', ['/d', '/c', 'echo out& echo err 1>&2& exit /b 3']);
            let out = '';
            let err = '';
            child.stdout.on('data', (chunk) => (out += chunk));
            child.stderr.on('data', (chunk) => (err += chunk));
            child.on('close', (code) => {
                console.log(JSON.stringify(out.trim()), JSON.stringify(err.trim()), code);
                const finding = spawn('findstr', ['e']);
                let found = '';
                finding.stdout.on('data', (chunk) => (found += chunk));
                finding.on('close', (code) => console.log(found.split(/\r?\n/).filter(Boolean).join(','), code));
                finding.stdin.write('pear\r\nfig\r\n');
                finding.stdin.end('apple\r\n');
            });
            "#
        ),
        "\"out\" \"err\" 3\npear,apple 0"
    );
}

#[test]
fn a_connection_waiting_to_hear_something_does_not_count_as_busy() {
    in_time("quiet", connection_waiting);
}

fn connection_waiting() {
    let dir = std::env::temp_dir().join(format!("rsui_js_net_quiet_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.js");
    std::fs::write(
        &file,
        r#"
        const net = require('net');
        const server = net.createServer((socket) => (globalThis.accepted = socket));
        server.listen(0, '127.0.0.1', () => {
            const client = net.connect(server.address().port, '127.0.0.1', () => console.log('connected'));
            globalThis.stop = () => {
                client.destroy();
                globalThis.accepted.destroy();
                server.close(() => console.log('stopped'));
            };
        });
        "#,
    )
    .unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    engine.run_file(&file).unwrap();
    let started = std::time::Instant::now();
    while !output.borrow().contains("connected")
        || engine
            .eval("globalThis.accepted === undefined", "check.js")
            .map(|value| engine.display(value))
            .as_deref()
            != Ok("false")
    {
        assert!(started.elapsed() < STUCK_AFTER, "it never connected");
        engine.run_jobs().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    engine.run_jobs().unwrap();
    assert!(engine.is_working());
    assert!(!engine.is_busy());
    engine.eval("globalThis.stop()", "stop.js").unwrap();
    engine.run_until_idle().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(output.borrow().contains("stopped"));
}

/// A server that only sends the rest once the client has seen the first part, which it can't if `fetch` waits for everything. Each prints what Node 24 prints for the same program.
#[test]
fn fetch_gives_the_first_part_of_a_body_before_the_server_has_sent_the_rest() {
    assert_eq!(
        run(
            "n1",
            r#"
            const http = require('http');
            let sawFirst;
            const firstSeen = new Promise((resolve) => (sawFirst = resolve));
            const server = http.createServer((request, response) => {
                response.writeHead(200, { 'Content-Type': 'text/plain' });
                response.write('first part;');
                console.log('server sent the first part');
                // If the client only hears once everything's come, it never hears the first part by itself.
                const late = setTimeout(() => {
                    console.log('the client did not see the first part by itself');
                    sawFirst();
                }, 5000);
                firstSeen.then(() => {
                    clearTimeout(late);
                    console.log('server sends the last part');
                    response.end('last part');
                });
            });
            server.listen(0, '127.0.0.1', async () => {
                const response = await fetch('http://127.0.0.1:' + server.address().port + '/');
                console.log('headers are in', response.status, response.headers.get('content-type'), response.bodyUsed);
                const reader = response.body.getReader();
                const decoder = new TextDecoder();
                const first = await reader.read();
                console.log('client read', decoder.decode(first.value), first.value instanceof Uint8Array);
                sawFirst();
                let rest = '';
                for (;;) {
                    const { value, done } = await reader.read();
                    if (done) break;
                    rest += decoder.decode(value);
                }
                console.log('client read', rest, response.bodyUsed);
                server.close(() => console.log('closed'));
            });
            "#
        ),
        "server sent the first part\n\
         headers are in 200 text/plain false\n\
         client read first part; true\n\
         server sends the last part\n\
         client read last part true\n\
         closed"
    );
}

/// What an AI SDK does with a reply that streams: each event is seen before the server sends the next. Each prints what Node 24 prints for the same program.
#[test]
fn server_sent_events_arrive_one_at_a_time_through_a_text_decoder() {
    assert_eq!(
        run(
            "n2",
            r#"
            const http = require('http');
            // The server sends each event once the client has seen the one before, or after a while if it hasn't, which says so.
            let heard = () => {};
            const server = http.createServer((request, response) => {
                response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
                response.flushHeaders();
                let sent = 0;
                const send = () => {
                    sent += 1;
                    console.log('server sends event', sent);
                    response.write(`event: delta\ndata: {"n":${sent}}\n\n`);
                    if (sent === 3) {
                        response.end();
                        return;
                    }
                    const late = setTimeout(() => {
                        console.log('the client did not see event', sent, 'by itself');
                        next();
                    }, 5000);
                    const next = () => {
                        clearTimeout(late);
                        heard = () => {};
                        send();
                    };
                    heard = next;
                };
                send();
            });
            server.listen(0, '127.0.0.1', async () => {
                const response = await fetch('http://127.0.0.1:' + server.address().port + '/events');
                console.log(response.status, response.headers.get('content-type'));
                let buffer = '';
                for await (const text of response.body.pipeThrough(new TextDecoderStream())) {
                    buffer += text;
                    for (let end = buffer.indexOf('\n\n'); end >= 0; end = buffer.indexOf('\n\n')) {
                        const fields = Object.fromEntries(buffer.slice(0, end).split('\n').map((line) => line.split(': ')));
                        buffer = buffer.slice(end + 2);
                        console.log('client sees', fields.event, JSON.parse(fields.data).n);
                        heard();
                    }
                }
                console.log('stream ended', JSON.stringify(buffer));
                server.close(() => console.log('closed'));
            });
            "#
        ),
        "server sends event 1\n\
         200 text/event-stream\n\
         client sees delta 1\n\
         server sends event 2\n\
         client sees delta 2\n\
         server sends event 3\n\
         client sees delta 3\n\
         stream ended \"\"\n\
         closed"
    );
}

/// Stopping a request partway, while it's waiting for headers and while its body is being read, from an `AbortSignal` and by cancelling its body. Each prints what Node 24 prints for the same program.
#[test]
fn aborting_or_cancelling_a_fetch_stops_it_and_closes_the_connection() {
    assert_eq!(
        run(
            "n3",
            r#"
            const http = require('http');
            let closedSockets = 0;
            let sawClose = () => {};
            let lateArrived = () => {};
            const server = http.createServer((request, response) => {
                request.socket.on('close', () => {
                    closedSockets += 1;
                    console.log('server saw', request.url, 'close');
                    sawClose();
                });
                if (request.url === '/late') {
                    // The headers never come, so the request is stopped while it's waiting for them.
                    lateArrived();
                    return;
                }
                response.writeHead(200);
                response.write('the start');
            });
            const closing = () => new Promise((resolve) => (sawClose = resolve));
            server.listen(0, '127.0.0.1', async () => {
                const base = 'http://127.0.0.1:' + server.address().port;
                const controller = new AbortController();
                const response = await fetch(base + '/abort', { signal: controller.signal });
                const reader = response.body.getReader();
                console.log('read', new TextDecoder().decode((await reader.read()).value));
                let closed = closing();
                const pending = reader.read();
                controller.abort();
                await pending.then(() => console.log('read after abort'), (error) => console.log('read rejects with', error.name));
                await reader.closed.catch((error) => console.log('closed rejects with', error.name));
                await closed;

                closed = closing();
                const other = await fetch(base + '/cancel');
                const otherReader = other.body.getReader();
                await otherReader.read();
                await otherReader.cancel('enough');
                console.log('cancelled', other.bodyUsed);
                await closed;

                closed = closing();
                const early = new AbortController();
                // Stopped once the server has it, rather than after 50 ms: on a busy machine that was before, the server never
                // saw a connection close, and this waited for it for ever.
                lateArrived = () => setTimeout(() => early.abort(new Error('gave up')), 10);
                const waiting = fetch(base + '/late', { signal: early.signal });
                await waiting.then(() => console.log('fetch kept'), (error) => console.log('fetch rejects with', error.message));
                await closed;
                await fetch(base + '/never', { signal: AbortSignal.abort() }).catch((error) => console.log('already aborted', error.name));
                console.log('sockets closed', closedSockets);
                server.close(() => console.log('closed'));
            });
            "#
        ),
        "read the start\n\
         read rejects with AbortError\n\
         closed rejects with AbortError\n\
         server saw /abort close\n\
         cancelled true\n\
         server saw /cancel close\n\
         fetch rejects with gave up\n\
         server saw /late close\n\
         already aborted AbortError\n\
         sockets closed 3\n\
         closed"
    );
}

/// Node's `http` client on the same request, and the server sending what it writes in chunks, as it writes them. Each prints what Node 24 prints for the same program.
#[test]
fn http_get_hears_each_part_as_it_comes_and_pausing_holds_the_rest() {
    assert_eq!(
        run(
            "n4",
            r#"
            const http = require('http');
            let heard = () => {};
            const server = http.createServer((request, response) => {
                if (request.url === '/forever') {
                    response.writeHead(200);
                    response.write('hello');
                    request.socket.on('close', () => console.log('server saw the client go'));
                    return;
                }
                response.writeHead(200, { 'Content-Type': 'text/plain' });
                response.write('part 1;');
                console.log('server sent part 1');
                const late = setTimeout(() => {
                    console.log('the client did not see part 1 by itself');
                    heard();
                }, 5000);
                heard = () => {
                    clearTimeout(late);
                    heard = () => {};
                    console.log('server sends parts 2 and 3');
                    response.write('part 2;');
                    setTimeout(() => response.end('part 3;'), 20);
                };
            });
            server.listen(0, '127.0.0.1', () => {
                const base = 'http://127.0.0.1:' + server.address().port;
                http.get(base + '/parts', (response) => {
                    console.log('status', response.statusCode, response.headers['transfer-encoding'], response.complete);
                    response.setEncoding('utf8');
                    let paused = false;
                    response.on('data', (chunk) => {
                        console.log('client data', chunk, paused);
                        if (chunk === 'part 1;') {
                            response.pause();
                            paused = true;
                            heard();
                            setTimeout(() => {
                                console.log('resume');
                                paused = false;
                                response.resume();
                            }, 200);
                        }
                    });
                    response.on('end', () => {
                        console.log('end', response.complete);
                        const request = http.get(base + '/forever', (cut) => {
                            cut.on('data', (chunk) => {
                                console.log('forever data', String(chunk));
                                request.destroy();
                            });
                            cut.on('aborted', () => console.log('response aborted'));
                            cut.on('error', (error) => console.log('response error', error.message, error.code));
                            cut.on('close', () => {
                                console.log('response close');
                                setTimeout(() => server.close(() => console.log('closed')), 100);
                            });
                        });
                        request.on('close', () => console.log('request close'));
                    });
                });
            });
            "#
        ),
        "server sent part 1\n\
         status 200 chunked false\n\
         client data part 1; false\n\
         server sends parts 2 and 3\n\
         resume\n\
         client data part 2; false\n\
         client data part 3; false\n\
         end true\n\
         forever data hello\n\
         response aborted\n\
         request close\n\
         response error aborted ECONNRESET\n\
         response close\n\
         server saw the client go\n\
         closed"
    );
}
