// `updates` in `rsui/desktop`: a packed program replacing itself with a newer version of itself, only ever with exactly the file
// that was meant, which is what the SHA-256 is for.

const { host, resolved } = require('rsui/desktop/core');

function sha256Of(file) {
    return require('crypto').createHash('sha256').update(require('fs').readFileSync(file)).digest('hex');
}

function checkedHash(sha256) {
    if (typeof sha256 !== 'string' || !/^[0-9a-fA-F]{64}$/.test(sha256)) {
        throw new TypeError('an update needs the SHA-256 of its file, as a program should never replace itself with just anything');
    }
    return sha256.toLowerCase();
}

/// Downloads the new version to a file of its own, and checks it's the one that was meant. It's that file's path. `onProgress`
/// is told `{ received, total }` as it comes, with `total` null if the server doesn't say.
async function download({ url, sha256, onProgress } = {}) {
    const wanted = checkedHash(sha256);
    const response = await fetch(String(url));
    if (!response.ok) {
        throw new Error('The update could not be downloaded: ' + response.status);
    }
    const fs = require('fs');
    const path = require('path');
    const total = Number(response.headers.get('content-length')) || null;
    let bytes;
    if (typeof onProgress === 'function' && response.body?.getReader) {
        const reader = response.body.getReader();
        const parts = [];
        let received = 0;
        onProgress({ received, total });
        for (;;) {
            const { done, value } = await reader.read();
            if (done) {
                break;
            }
            parts.push(value);
            received += value.length;
            onProgress({ received, total });
        }
        bytes = Buffer.concat(parts.map((part) => Buffer.from(part)));
    } else {
        bytes = Buffer.from(await response.arrayBuffer());
    }
    const folder = fs.mkdtempSync(path.join(require('os').tmpdir(), 'rsui-update-'));
    const file = path.join(folder, 'update.exe');
    fs.writeFileSync(file, bytes);
    const got = sha256Of(file);
    if (got !== wanted) {
        fs.rmSync(folder, { recursive: true, force: true });
        throw new Error('The download is not the update that was meant: its SHA-256 is ' + got + ', and not ' + wanted);
    }
    return file;
}

/// Replaces this program with this file, as long as it's exactly the one that was meant. It's what runs the next time the program
/// starts, and `app.relaunch()` does that right away.
function install(file, sha256) {
    const wanted = checkedHash(sha256);
    const place = resolved(file);
    const got = sha256Of(place);
    if (got !== wanted) {
        throw new Error('That file is not the update that was meant: its SHA-256 is ' + got + ', and not ' + wanted);
    }
    host.replaceProgram(place);
}

/// Whether one version comes after another, as `1.10.0` does after `1.9.2`, with what's after a `-` coming before the version itself.
function newer(version, than) {
    const parts = (text) => String(text).replace(/^v/i, '').split('-');
    const [mainA, extraA] = parts(version);
    const [mainB, extraB] = parts(than);
    const a = mainA.split('.').map((one) => parseInt(one, 10) || 0);
    const b = mainB.split('.').map((one) => parseInt(one, 10) || 0);
    for (let at = 0; at < Math.max(a.length, b.length); at += 1) {
        if ((a[at] ?? 0) !== (b[at] ?? 0)) {
            return (a[at] ?? 0) > (b[at] ?? 0);
        }
    }
    if (extraA === undefined || extraB === undefined) {
        return extraA === undefined && extraB !== undefined;
    }
    return extraA > extraB;
}

/// Asks whether there's a newer version, from a JSON file at `url` that says `{ version, url, sha256, notes }`. It's that, with
/// `available` saying whether it's newer than this one, which `download` and `install` take.
async function check(url) {
    const response = await fetch(String(url));
    if (!response.ok) {
        throw new Error('Whether there is an update could not be found out: ' + response.status);
    }
    const said = await response.json();
    if (typeof said?.version !== 'string' || typeof said?.url !== 'string') {
        throw new Error('What says whether there is an update needs a version and a url');
    }
    const now = require('rsui/desktop/app').app.getInfo().version;
    return { available: newer(said.version, now), version: said.version, url: new URL(said.url, String(url)).href, sha256: said.sha256, notes: said.notes ?? '' };
}

/// Says this version of the program works. A packed program that's just been updated has to, or the next time it starts, the
/// version before it is put back.
function confirmHealthy() {
    host.updateConfirmHealthy();
}

/// Whether this is a new version that hasn't said it works yet.
const waitingToConfirm = () => host.updateUnconfirmed();

const updates = { check, download, install, confirmHealthy, waitingToConfirm, __newer: newer };

module.exports = { updates };
