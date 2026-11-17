// `sound` in `rsui/desktop`: playing a sound file, and the sounds the system makes.

const { host, action, forget, resolved } = require('rsui/desktop/core');

/// A sound's file, as a real file. Those inside a packed program have to be put somewhere the system can read them from.
function soundFile(file) {
    const place = resolved(file);
    if (!host.isPacked()) {
        return place;
    }
    const fs = require('fs');
    const path = require('path');
    const folder = path.join(require('os').tmpdir(), 'rsui-sounds', String(process.pid));
    fs.mkdirSync(folder, { recursive: true });
    const copy = path.join(folder, path.basename(place));
    fs.writeFileSync(copy, fs.readFileSync(place));
    return copy;
}

/// One sound file being played. It's opened when it's first played, and let go of once it's played to its end.
class Playback {
    #file;
    #id = null;
    #ended = 0;
    #volume = 1;
    #muted = false;
    #fromStart = true;
    #playing = false;

    constructor(file, { volume = 1, loop = false, onEnded = null } = {}) {
        this.#file = file;
        this.loop = loop === true;
        this.onEnded = onEnded;
        this.volume = volume;
    }

    get file() {
        return this.#file;
    }

    get playing() {
        return this.#playing;
    }

    get volume() {
        return this.#volume;
    }

    set volume(value) {
        this.#volume = Math.min(1, Math.max(0, Number(value)));
        this.#applyVolume();
    }

    get muted() {
        return this.#muted;
    }

    set muted(on) {
        this.#muted = on === true;
        this.#applyVolume();
    }

    /// How long it is, in milliseconds, once that's known.
    get duration() {
        return this.#id === null ? null : host.soundAsk(this.#id, 'length');
    }

    /// Where it's up to, in milliseconds.
    get position() {
        const at = this.#id === null ? null : host.soundAsk(this.#id, 'position');
        return at ?? 0;
    }

    seek(milliseconds) {
        if (this.#id === null) {
            this.#open();
        }
        host.soundSeek(this.#id, Number(milliseconds));
        this.#fromStart = false;
    }

    play() {
        if (this.#id === null) {
            this.#open();
        }
        host.soundPlay(this.#id, this.#fromStart, this.#ended);
        this.#fromStart = false;
        this.#playing = true;
        return this;
    }

    pause() {
        if (this.#id !== null) {
            host.soundDo(this.#id, 'pause');
        }
        this.#playing = false;
    }

    /// Stops it and lets go of the file. Playing it again starts from the beginning.
    stop() {
        this.#close();
        this.#playing = false;
        this.#fromStart = true;
    }

    #applyVolume() {
        if (this.#id !== null) {
            host.soundVolume(this.#id, this.#muted ? 0 : this.#volume);
        }
    }

    #open() {
        this.#id = host.soundOpen(soundFile(this.#file));
        this.#ended = action(() => this.#reachedEnd());
        this.#fromStart = true;
        this.#applyVolume();
    }

    #close() {
        if (this.#id !== null) {
            host.soundDo(this.#id, 'close');
            forget([this.#ended]);
            this.#id = null;
        }
    }

    #reachedEnd() {
        if (this.loop) {
            host.soundPlay(this.#id, true, this.#ended);
            return;
        }
        // Lots of these get made, each playing once, and what's playing them is worth having back.
        this.#close();
        this.#playing = false;
        this.#fromStart = true;
        this.onEnded?.();
    }
}

/// Plays a sound file, MP3 or WAV, and gives back what's playing it.
function play(file, options) {
    return new Playback(String(file), options).play();
}

/// Makes the sound the system has for this kind of message: 'info', 'warning', 'error' or 'question', or its plain one.
function beep(kind = '') {
    host.beep(String(kind));
}

module.exports = { sound: { play, beep }, Playback };
