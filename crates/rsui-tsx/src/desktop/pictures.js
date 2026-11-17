// `files` and `capture` in `rsui/desktop`: pictures of what's in files, as Explorer shows them, and of what's on the screen.

const { host, resolved } = require('rsui/desktop/core');

/// A picture as Rust gives it, `width height base64`, as `{ width, height, png, url }`, or `null` for none.
function picture(text) {
    if (text === null || text === undefined) {
        return null;
    }
    const [width, height, data] = String(text).split(' ');
    return {
        width: Number(width),
        height: Number(height),
        png: new Uint8Array(Buffer.from(data, 'base64')),
        url: 'data:image/png;base64,' + data,
    };
}

/// What `ask` gives, as a promise of a picture, which is rejected if asking throws.
function asked(ask) {
    try {
        return Promise.resolve(ask()).then(picture);
    } catch (error) {
        return Promise.reject(error);
    }
}

const files = {
    /// What Explorer shows of what's in a file, as a photo's or a document's first page, at most `size` pixels across:
    /// `{ width, height, png, url }`, with `png` a PNG file's bytes and `url` a `data:` URL of it. It's `null` for a file that
    /// has no thumbnail.
    thumbnail: (path, { size = 256 } = {}) => asked(() => host.filePicture(resolved(path), Math.round(Number(size) || 256), false)),
    /// A file's icon, as Explorer shows it, at most `size` pixels across, the same way.
    icon: (path, { size = 32 } = {}) => asked(() => host.filePicture(resolved(path), Math.round(Number(size) || 32), true)),
};

/// What's on the screen in a rectangle, in physical pixels from the main screen's top left.
function region({ x = 0, y = 0, width, height } = {}) {
    return asked(() => host.captureScreen(Number(x), Number(y), Number(width), Number(height)));
}

const capture = {
    /// What's on a screen, by where it is in `displays.list()`, or the main one, as `{ width, height, png, url }`. While an app's
    /// being checked, it's said instead and the picture is blank.
    screen(displayIndex) {
        const list = host.displays();
        const display = displayIndex === undefined || displayIndex === null
            ? list.find((one) => one.primary) ?? list[0]
            : list[Number(displayIndex)];
        if (display === undefined) {
            return Promise.reject(new RangeError(`there's no screen ${displayIndex ?? ''}`.trim()));
        }
        return region(display);
    },
    /// What's on the screen in `{ x, y, width, height }`, the same way.
    region,
};

module.exports = { files, capture };
