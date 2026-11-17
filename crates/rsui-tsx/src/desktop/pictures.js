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

/// A folder's listing as Rust gives it, a line for each thing, as objects.
function listing(text) {
    const found = [];
    if (!text) {
        return found;
    }
    for (const line of String(text).split('\n')) {
        if (line === '') {
            continue;
        }
        const [name, flags, size, modified, created] = line.split('\t');
        const bits = Number(flags);
        found.push({
            name,
            folder: (bits & 1) !== 0,
            hidden: (bits & 2) !== 0,
            system: (bits & 4) !== 0,
            readonly: (bits & 8) !== 0,
            link: (bits & 16) !== 0,
            size: Number(size),
            modified: Number(modified),
            created: Number(created),
        });
    }
    return found;
}

/// A promise of what `ask` gives, which is rejected if asking throws.
function promised(ask, then) {
    try {
        return Promise.resolve(ask()).then(then);
    } catch (error) {
        return Promise.reject(error);
    }
}

const files = {
    /// What's in a folder, read in one pass on another thread, with each thing's size, times and attributes:
    /// `[{ name, folder, hidden, system, readonly, link, size, modified, created }]`, in the order Explorer sorts names.
    list: (folder) => promised(() => host.folderList(resolved(folder)), listing),
    /// The drives there are, as `[{ path, label, kind, total, free }]`.
    drives: () => promised(() => host.drives(), (text) => JSON.parse(text)),
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
