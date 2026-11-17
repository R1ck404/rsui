'use strict';
// What programs and packages expect to find that isn't part of the language, all together, for what asks for them that
// way. Each part is in its own module, so what only wants one of them doesn't compile the others.

const { URL, URLSearchParams } = require('internal/url');
const { Event, EventTarget, AbortController, AbortSignal } = require('internal/event-target');
const { Buffer } = require('internal/buffer');

module.exports = { URL, URLSearchParams, Event, EventTarget, AbortController, AbortSignal, Buffer };
