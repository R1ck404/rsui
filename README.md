# rsui

A native UI framework for Windows, written from scratch: its own text shaping, layout, GPU renderer, image decoders and
JavaScript engine. You write apps in Rust, or in TypeScript and JSX with hooks like React's. There is no web view.

Agents come first. Everything an app shows can be read and used as a tree with no window at all, and a person using a real
window gets the same app drawn by the GPU.

**Status:** version 0.1.0. Windows 10 or later only. The APIs can still change.

## Getting started

You need Windows, and [Rust](https://rustup.rs) 1.85 or later with the MSVC toolchain. You don't need Node or a browser, and the app you
make is a single program.

```
git clone https://github.com/R1ck404/rsui
cargo install --path rsui/crates/rsui-tsx --locked
rsui-tsx new my-app
cd my-app
rsui-tsx app.tsx
```

That opens a window, which updates when you save `app.tsx`. `rsui.d.ts` next to it is the whole API. `AGENTS.md` tells an agent how
to check its work with no window.
`rsui-tsx --pack my-app.exe app.tsx` makes one program you can hand to someone.

## An app

```tsx
import { useState } from 'rsui';

export const title = 'Counter';

export default function App() {
    const [count, setCount] = useState(0);
    return (
        <window padding={16} gap={8}>
            <text fontSize={20}>Clicked {count} times</text>
            <button key="add" paddingX={14} paddingY={7} background="#1f6feb" color="#ffffff" radius={6}
                onClick={() => setCount(count + 1)}>
                Add one
            </button>
        </window>
    );
}
```

`crates/rsui-tsx/rsui.d.ts` has the types for editors and `tsc`, with a description of everything. `crates/rsui-tsx/DESIGN.md`
explains how it works, how it differs from React, and how to keep a long list fast.

## For agents

Working on an app needs no window and no person.

- `rsui-tsx --png out.png app.tsx` draws the app to a file and prints its tree, with the name of every element and what it can do. Other
  windows the app opens are saved next to it, as `out.window-1.png` and so on.
- `rsui-tsx --do "click #add; type #name hello; expect \"hello\"; shot out.png" app.tsx` uses the app the way a person would, in one
  command with no window. Clicks land where things really are, keys can be held (`click #row ctrl`) and the pointer can move
  (`move #chart`), so disabled or covered elements stay that way. Each step reports what it changed. The command exits with 1 at the
  first step that fails and says why and what was there. Steps can live in a file (`--do @steps.txt`), which makes them a test. The app
  reads and writes its usual files, but no dialog or link ever opens.
- `rsui-tsx --agent app.tsx` serves the app over the Model Context Protocol, with no window, for a client to keep open while it works.
  The client can read the tree, act on it, press keys, close windows and take pictures, in every window the app opens. A saved file is
  running by the next tool call, with the app's state kept. Set it up like any other server that starts as a program:

  ```json
  { "mcpServers": { "my-app": { "command": "rsui-tsx", "args": ["--agent", "app.tsx"] } } }
  ```

- `crates/rsui-tsx/tests/hands/mod.rs` does the same from a Rust test.
- `tsc` with `crates/rsui-tsx/rsui.d.ts` finds most mistakes before anything runs. A function passed to an event that doesn't exist is an
  error that lists the events that do.
- An error while rendering shows where it happened, and the rest of the app carries on.

## Code written for the web

`import ... from 'react'` works, and so does JSX written for a page, with nothing to convert.

- **React 19:** class components, error boundaries, `Suspense`, every hook, `use` and actions. `<form action={fn}>` works with
  `useFormStatus`, `requestSubmit`, Enter and `FormData`.
- **Markup and CSS:** `className`, `import './app.css'`, CSS modules, and `<style>` or `<link>` anywhere in `<head>`, applied in the order they
  appear. Tables are laid out as tables. The same matcher serves CSS and `querySelector`, `matches` and `closest`, so `li + li`,
  `:nth-child(2 of .row)`, `:has()` and Tailwind's `group-hover:` and `peer-checked:` behave the same in both.
- **Events and the DOM:** events carry where the pointer was and which keys were held. Keys reach `keydown` handlers before a field takes
  them. Clipboard, text composition, `tabIndex`, drag and drop (including from other programs), `document.addEventListener`,
  `scrollTop`, `scrollIntoView` and `scroll` events all work, and so do refs to elements, `getBoundingClientRect` and
  `getComputedStyle`. `<dialog>` has `showModal()`. `scrollbar-color` and `scrollbar-width` are supported.
- **Browser state:** `localStorage` works, and `history` and `location` are kept in memory, so react-router's `BrowserRouter` runs as it is.
  A rejected promise with nothing to catch it is logged and delivered to `unhandledrejection`, as in a browser.
- **Libraries from npm** run as they are. Packages are found the way a bundler for a browser finds them, using their `browser` builds, and
  `document`, `window` and the rest of the DOM load only when something touches them, so an app that never touches the DOM doesn't pay for
  it. Redux Toolkit, react-redux, zustand, TanStack Query (with Table and Virtual), react-i18next, react-router, Radix, Headless UI,
  React Aria, Floating UI, motion, react-hook-form, react-select, styled-components, emotion, sonner and recharts have all been run.
  So have the drag-and-drop libraries react-beautiful-dnd, react-dnd, dnd-kit and @hello-pangea/dnd.

What differs from a browser is listed in [`crates/rsui-tsx/DESIGN.md`](crates/rsui-tsx/DESIGN.md).

## Desktop features

`rsui/desktop` has one object for each part of a desktop app, loaded the first time it's used: `shell`, `clipboard`, `secrets`,
`dialogs`, `notifications`, `tray`, `taskbar`, `shortcuts`, `sound`, `app`, `protocols`, `fileAssociations`, `displays`, `system`,
`power`, `network`, `drag`, `menus`, `storage`, `updates`, `files` (thumbnails and icons as Explorer shows them), `capture` (the
screen), `audio` (the sound devices) and `media` (the media keys). `taskbar` also has the jump list, and `dialogs` can ask for a color
or a font.

Windows are controlled through `mainWindow` and `useCurrentWindow()`: title, menu bar, minimized, maximized, fullscreen, always on top,
frameless, opacity, size limits, icon, taskbar button, the buttons under the taskbar preview, and Windows 11's backdrop. `getState()`
and `onStateChange()` report how a window is, which `useWindowState()` renders with. `<Window rememberPlacement="settings">` opens where
it was last time.

Where a page and rsui both offer a way to reach the same thing, they share one implementation underneath, so what either sets, the
other reads, and both hear about changes:

| Web | rsui |
|---|---|
| `document.title` | `win.setTitle()` |
| `localStorage` | `storage` |
| `innerWidth` | `getState()` |
| `document.hidden`, `visibilitychange` | whether the window is shown |
| `matchMedia` | the system's settings |
| `navigator.onLine` | `network` |
| `navigator.language` | `system.locale()` |
| `navigator.mediaDevices` | `audio` |
| `navigator.mediaSession` | `media` |

With `--do` and `--agent`, every desktop action is reported rather than performed, so nothing opens. Set `RSUI_SANDBOX` to a folder and
anything that would persist in the user's settings or credentials goes there instead.

## What it costs

Measured on one machine, a Ryzen 7800X3D with an RTX 5070, on Windows 11.

| | |
|---|---|
| Window on screen after starting, for an app in TSX | about 21 ms |
| Peak RAM across the example apps | about 19 MB |
| RAM while in use | 5 MB, of which under 2 MB is the program's alone |
| RAM while idle | about 0.2 MB |
| CPU while idle, about 10 seconds after the last event | none |
| One frame of a typical window | about 50 us of CPU and about 120 us of GPU |
| Laying out 10,000 rows | about 3 ms |
| Program size | 1.4 MB in Rust, 4.4 MB with the JavaScript engine (the app's code is added to that when packed into one file) |

About 50 MB is committed while a window has the GPU. That counts against the system's limit but mostly isn't in RAM, nearly all of it is
the NVIDIA driver's, and it is released when a window has been idle long enough. `crates/rsui-core/PERFORMANCE.md` has more, including
what was tried and didn't work.

The JavaScript engine passes 87.4% of Test262 and 99.5% of the `Date` tests. Most of the rest is features it doesn't have yet, which
`crates/rsui-js/DESIGN.md` lists. 135 popular npm packages behave as they do on Node, including axios, zod, cheerio, mathjs, d3 and
jszip, and so do `fs`, `crypto`, `stream`, `child_process` and the rest of the Node modules it implements. `rsui-tsx` includes the
engine's baseline JIT, which compiles the functions an app runs most to x86-64 machine code after a few thousand calls. `RSUI_JIT=0`
turns it off, and `crates/rsui-js/JIT.md` describes it.

## The crates

| Crate | What's in it |
|---|---|
| `rsui` | Windows, input, focus, popups and accessibility: what most programs use |
| `rsui-tsx` | Apps in TypeScript and JSX, with hooks and live reload |
| `rsui-js` | The JavaScript engine, which reads TypeScript as it is |
| `rsui-runtime` | Components, state and effects |
| `rsui-core` | The tree of elements, and finding what changed |
| `rsui-layout` | Flexbox, scrolling and long lists |
| `rsui-text` | Fonts, shaping for every script, line breaking and glyphs |
| `rsui-render` | Direct3D 11, and turning a tree into what to draw |
| `rsui-image` | PNG, JPEG, WebP and animated images |
| `rsui-alloc` | An allocator for this kind of work |
| `rsui-headless` | Running an app with no window, for tests and agents |
| `rsui-agent` | Tools over the Model Context Protocol that let an agent operate an app |

## Your own Rust program

Every program should include this line, since `rsui` is built around it:

```rust
#[global_allocator]
static ALLOCATOR: rsui::UiAlloc = rsui::UiAlloc;
```

Copy `.cargo/config.toml` from this repo too. It links the part of the C runtime that a clean Windows install may lack into your
program, for 23 KB. Without it, the program won't start on some machines.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
Unless you say otherwise, any contribution you submit for inclusion is dual licensed as above, with no additional terms.
