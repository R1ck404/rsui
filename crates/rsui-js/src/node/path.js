'use strict';
// Node's `path`, for packages that expect it. It's made in Rust (`builtins/host/path.rs`), which gives anything that
// isn't text to the JavaScript this used to be (`internal/path-js`), only compiled then.
module.exports = __fs.pathModule(process.platform === 'win32');
