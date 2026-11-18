# rsui-tsx

Writes rsui apps in TypeScript and JSX, with hooks like React's, on rsui's own JavaScript engine. There's no webview, and
no DOM. A tag is an element of rsui's tree, and a component is a component of its runtime.

```tsx
import { useState } from 'rsui';

export const title = 'Counter';

export default function App() {
    const [count, setCount] = useState(0);
    return (
        <window padding={24} gap={12} align="start">
            <text fontSize={20}>Clicked {count} times</text>
            <button paddingX={14} paddingY={8} background="#1f6feb" color="white" radius={6} onClick={() => setCount(count + 1)}>
                Add one
            </button>
        </window>
    );
}
```

Make one with `rsui-tsx new my-app`. Run it with `cargo run --release -p rsui-tsx -- app.tsx`, then change the file and save it. What's on screen changes, and
the app keeps its state. `--once` leaves out the watching, and `--png out.png` draws a frame to a file and prints the tree.

From Rust, it's `rsui_tsx::run("app.tsx", Options::default())`, or `rsui_tsx::load` and `rsui_tsx::root` to mount it in an
`App` or a `Headless` of your own.

## Handing it to someone

`rsui-tsx --pack app.exe app.tsx` makes one program of an app, about as big as `rsui-tsx` itself plus the app's code. It runs the
main file once to see what it loads, and takes that, with the whole of any package it's from, as a package can load more of itself
later. They go on the end of the program, and are read from there as they're needed, so they take no memory until then. To the
app, they look as if they're next to the program, which is also where anything it reads for itself is looked for. Pictures and other
files it reads for itself aren't taken along, so they go next to it. A program made this way doesn't bring up a console, shows
what's wrong in a message if it can't start, and `--icon app.ico` gives it an icon.

## What there is

**Tags:** `window`, `group`, `row`, `column`, `text`, `span`, `button`, `link`, `input`, `checkbox`, `item`, `image`, `dialog`, `menu`,
`menuitem`, `scroll` and `list`. Anything else becomes a custom role. Text inside `text`, `button`, `link`, `menuitem`, `item`
or `checkbox` is its name, which is what agents and screen readers see. `name` sets that directly.

**Color** goes down: text with no `color` of its own takes it from what it's in, and so does a shape's `currentColor`, and the
thumbs of what scrolls. Nothing else does, and neither do fonts, unless it's parts of a paragraph.

**Paragraphs:** text with other text inside it wraps as one piece, and each part can look different:

```tsx
<text>
    This is <span fontWeight="bold">bold</span>, this is <span fontFamily="Consolas" background="#eaeef2" radius={3}>code</span>,
    and this is <link color="#0969da" underline onClick={open}>a link</link>.
</text>
```

Each part takes what it doesn't set from what it's in, and they can be nested, or be components or lists of parts. Size, weight,
style, font, color, underline, `background`, `radius` and `hover` all work on a part. So does `onClick`, on the words themselves
across however many lines they end up on. What doesn't apply to words doesn't apply to a part: it has no size of its own, and
`paddingX` only says how far its background reaches past its words, without moving them. A paragraph can be kept to one line, but
doesn't get an ellipsis yet. Anything but text inside `text` makes it an ordinary box again.

`selectable` on a `text` lets the pointer select it: dragging, two clicks for a word and three for all of it, with ctrl+C to copy and
ctrl+A for the rest. It's off unless asked for, as labels on a button or a heading aren't what people want to select by accident.
What's selected stays within one `text`.

**Style** goes straight on an element, or in `style` to share it: `direction`, `width` and `height` (a number, or `'50%'`),
`minWidth`, `maxWidth`, `minHeight`, `maxHeight`, `flex`, `grow`, `shrink`, `gap`, `padding`, `paddingX`, `paddingY`,
`paddingTop` and so on, `align`, `alignSelf`, `justify`, `overlay`, `background`, `hover`, `pressed`, `color`, `borderColor`,
`borderWidth`, `radius`, `opacity`, `fontSize` and `fontWeight`. A color is `'#rgb'`, `'#rrggbb'`, `'#rrggbbaa'`, `'white'`,
`'black'`, `'transparent'` or a number like `0x1f6feb`. Any other prop goes on the element with its name in `snake_case`.

**Handlers:** `onClick`, `onDoubleClick`, `onContextMenu`, `onDrag`, `onFocus`, `onToggle`, `onChange`, `onScroll`, `onKeyPress`,
`onResize`, `onDropFiles` and `onDismiss`. A function given to any other `on` name is an error that lists these, as otherwise it would just never
be called. `onClick` is given where the pointer was (`clientX`, `offsetX` and the rest), which button, how many clicks and what was held
(`ctrlKey`, `shiftKey`, `altKey` and `metaKey`), and `onKeyPress` hears F1 to F24, insert and alt as well, as in `"alt+s"` or `"f2"`.
A click stops where it lands, unless its handler calls `e.passOn()`, which sends it to the nearest `onClick` around it, and
`onClickCapture` hears every click on anything inside it first, so a row can be chosen wherever it's clicked.

These are given events as the web has them, and are as they are on a page's elements (see "Writing an app as for a page"):
`onMouseEnter`, `onMouseLeave`, `onPointerEnter`, `onPointerLeave`, `onPointerDown`, `onPointerUp`, `onPointerMove`, `onMouseDown`,
`onMouseUp`, `onMouseMove`, `onWheel`, `onKeyDown`, `onKeyUp`, their `Capture` forms, `onAuxClick`, `onPaste`, `onCopy`, `onCut`,
`onCompositionStart`, `onCompositionUpdate`, `onCompositionEnd`, `onBlur`, and, with `draggable`, `onDragStart`, `onDragEnter`, `onDragOver`,
`onDragLeave`, `onDrop` and `onDragEnd`. `onChange` is still given the new value, and
`onFocus`, `onKeyPress`, `onDoubleClick`, `onContextMenu` and `onDrag` are as they were, with the event after what they're given.
`tabIndex` on anything makes it something tab reaches, the lowest above 0 first, then the rest in order, and -1 keeps tab from it,
though it can still be clicked to.

**Hooks:** `useState`, `useReducer`, `useRef`, `useMemo`, `useCallback`, `useEffect`, `useLayoutEffect`, `createContext` with
`useContext`, and `useAnimated`, which is a number that moves smoothly to where it's told.

**From the system:** `useColorScheme()` is `'dark'` or `'light'` and follows the system, along with the title bar. `shell.open(url)`,
from `rsui/desktop`, opens http, https and mailto, and refuses any other address, as a link in a program can come from anywhere. `onDropFiles` on the
`window` takes files dragged in from elsewhere, and is given an array of paths. On anything inside it, that gets the ones dropped
there first.

**Shapes:** `Svg` holds `Path`, `Line`, `Polyline`, `Polygon`, `Rect`, `Circle`, `Ellipse` and `G`, for icons and charts:

```tsx
<Svg viewBox="0 0 24 24" width={16} height={16} color={INK} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
    <Path d="M5 12l5 5L20 7" />
</Svg>
```

All of a path's commands work, and a filled path follows the non-zero rule, which is what icons are mostly drawn for. There's no
`fill-rule`, gradients, dashes, or miter joins: corners are round. It's drawn once at the size it's shown, and kept on the GPU as a
picture, so a complicated shape that isn't changing costs what a picture costs. One that changes each frame gets drawn each frame.

**Widgets:** `Dialog`, `MenuButton`, `Select`, `TextInput`, `Checkbox`, `Switch`, `RadioGroup`, `Slider`, `Tabs`, `ProgressBar` and `Spinner`, all
written in the layer with what any program has. `disabled` works on anything: it can't be clicked or have the keyboard, and it fades.
A `TextInput` can be a `password`, `readOnly` or have a `maxLength`.

**The window:** `export const title`, `export const size = [1100, 700]`, and `export const minSize = [600, 400]` for how small it can be made.
`onClose` on the `window` is asked when its close button's clicked, for something like saving first, and `mainWindow.close()` then closes it.

**More windows:** `{open && <Window title="Settings" size={[400, 300]} onClose={() => setOpen(false)}>...</Window>}` opens one for as long as
it's there. What's inside is part of the app like anything else, with the same state and context, and it changes along with what's around
it. `modal` stops the main window being used until it's gone, and `owned={false}` lets it go behind the main window. Each has an `App` of its
own underneath, with its own layout, focus and GPU resources, all on one thread, so a window that isn't open costs nothing, and only what
changed in a window is drawn. Closing the main window ends the program, and everything else goes with it.

**A long list:** `<list count={1000000} rowHeight={30} renderRow={(index) => <Row index={index} />} />` only builds the rows in
view. Keep `renderRow` outside the component, or the rows all render again whenever it does. It's as big as layout makes it, as
anything else is, with `grow`, a share, all four sides of `position="absolute"`, or a page's flex box around it, and how many
rows fit is measured from that as it's laid out, so `visible` is only a guess for before then and nothing has to say how tall it
is. With `itemWidth` it's a grid that wraps by itself, as a folder's icons do: each row holds as many items that wide as fit
across, with its `gap` between them, and `renderRow` is given an item's index. `onResize` is told how many rows fit and how many
items across, the first time and whenever that changes, as the keyboard needs for going down a row or a page, and it's told
while the window's laid out, so what it changes is drawn in the same frame. Its other handlers, as `onClick`, hear what happens
on it below its rows, and its `scroll` is its own.

`rsui.d.ts` has the types for editors and `tsc`. Running an app never reads it, as the engine removes types as it parses them.
`tsconfig.json` needs `"jsx": "react"`, `"jsxFactory": "__jsx"` and `"jsxFragmentFactory": "__Fragment"`.

## How it works

`__jsx(type, props, children, mayHaveKey)` only describes what's wanted, in a plain object. `children` is one array, and the last two are left out when there's nothing to say, which is what JSX compiles to. When a component returns, Rust walks that description, making real
elements, and mounting each component in it through `Cx::child_of_kind`. So the runtime decides what renders, and everything
it does for an app in Rust happens here too: a child with the same props doesn't render, and only what changed reaches layout
and the screen.

Hooks live in JavaScript (`src/layer.js`), in an array on an object that belongs to each mounted component. That keeps Rust's
side the same whatever a component does, and it means breaking the rules of hooks is an error shown where it happened, rather
than a panic. Changing state tells Rust which component changed, and Rust marks it as dirty the next time it has the chance.
That's at the end of whatever JavaScript was running: a handler, an effect or a timer.

A JavaScript timer waits on the runtime's own timer, so a waiting app has no thread awake for it. Promises run when
whatever's running finishes. JavaScript runs on one thread, which is the one the window is on.

A component skips rendering when it's given the same props object, or props that are the same one level down and are all plain
values: numbers, strings, booleans. One that an app's own JSX made also skips when what it's given does the same as last time,
handlers made in place and all, with no `memo` or `useCallback` needed: see below. Anything else given an object, an array or a
function renders again whenever what it's in renders, as in React, unless it's wrapped in `memo` (see `JsProps` in bridge.rs).

### Not rendering what would come out the same

React Compiler keeps what a component makes, as the JSX elements, the handlers and the objects in them, until what each is made
from changes, so that a child given what it was given before has the same element, and React doesn't render it. An app gets
that without writing `memo`, `useMemo` or `useCallback`. rsui does the same thing the other way round: rather than keeping
what's made, it notices that what's made again is the same, when the child's props are compared.

- **What's the same.** `Vm::props_equivalent` (rsui-js `embed.rs`) says two props objects are when each prop is the same value, or a
  function made from the same code that sees the same values (`() => select(i)` made again with the same `select` and `i`,
  where what's in a `let`'s cell is compared, as each render has its own cells), or `bind` of the same thing with the same
  arguments, or a plain object or array made again holding what's the same in this way, as an element given as `children`, a
  `style={{ ... }}` or an object of handlers is. An instance of a class, a getter, a `Map` and the rest are only the same as
  themselves. It looks at no more than 256 objects and 6 levels down, and an array of 64 at most, so it costs a fraction of a
  render, and it never runs any of the app's code. The engine can do this because it knows each closure's code and what it
  captured, one value or cell for each variable it uses, and nothing else.
- **Where.** The parser makes JSX call `__jsxMemo` (layer.js), which marks a component's element, and the compiler has it call
  `__jsx` instead in a file that isn't the app's own, as one in `node_modules`, so a library's elements are as React has them
  (TanStack Table's `flexRender` gives cells the same `row` object, which says something else once it's selected). Only a marked
  element is compared this way, as React Compiler only keeps what it compiled, and only for a component written in the app's own
  files (each compiled function knows, in its proto's `says`): a library's components, and rsui's own page elements, which read
  the page's CSS as they render, render as they would in React. An element `__jsx` made says so (`memo: NOT_KEPT`), and is
  only the same as itself wherever it's found, as a cell made by `createElement` inside the app's `<div>`.
- **Being careful.** The same object is the same, as React's `memo` and React Compiler have it, as React's rules say what's given
  isn't changed in place, but not when it holds a function or `current`: a ref changes without a render, and an object with
  methods, as TanStack Table's `row`, can say something else when they're called, and a component can do either as it renders.
  (`memo` takes those to be the same, as React's does.) What a handler sees is only read when it's called, so it's the same as
  itself there. A class renders whenever its parent does. What reads a context or a store (`useSyncExternalStore`, and the
  libraries made on it) is told to render by the provider or the store, whatever its props are, and state still renders what
  it's in. What isn't covered is a component that reads something else that changes as it renders, as a variable outside it, or
  calls a function it's given that reads one, which React Compiler gets wrong the same way.
- **Saying no.** `'use no memo'` (or `'use no forget'`), as React Compiler has it, at the top of a file or of a function's body:
  the JSX there calls `__jsx`, and a component that says it renders whenever what it's in does (`Vm::says_no_memo`).

Why not compile it, as React Compiler does: that's working out, for each value in a component, what it's made from, and getting
that wrong shows something old, where comparing can only cost a render. Comparing also works for what React Compiler can't
keep, as elements made in a loop or in `renderRow`. What it doesn't save is the parent's own render: what the parent works out
is worked out again, which `useMemo` is still for when that's slow. A component that says `memo` compares in the same way, as it's
asking for the same thing, but without leaving out refs and objects with methods; given what wasn't the app's own JSX, `memo` is
as React's, comparing each prop as `Object.is` does.

What it comes to, moving the selection with the Down key in `demos/file-explorer` on a folder of 5,000 files, headless, with
each key's frames added up (the median of 40 keys, 3 runs of each):

| | Components rendered | Frame time |
|---|---|---|
| The demo as written, with `memo`, `useCallback` and its own `useStable` | 29 before, 23 now | 3.2 ms before, 2.9 ms now |
| A copy with all of those taken out | 432 before, 101 now | 26.6 ms before, 9.8 ms now |

What's left in the copy without them is the panel's own page elements (`Styled`), which aren't the app's and so render as they
would in React when the panel does, and the bar's buttons, whose handlers see what changed (what the panel goes back to, say),
as they would with React Compiler too.

A child is the same component as last time if it's in the same place, as in React: where it is in the array of children it's in, with `null`, `false` and `undefined` keeping their places, and the same for each array and element around it. An array or a fragment is one place, with its own places inside. A key takes the place of where it is in its array. A component of another type in a place, or one inside an element of another type, is a new one, and what was there is unmounted.

### Saving a file

A thread waits for the system to say something changed in the folder, using no CPU until it does. Then the main file and
everything it imports run again in the same engine. Components are told apart by name while this is going on, so a new
version of a component takes over from the old one in the same place, with the same hooks. If its hooks no longer line up,
that component starts again. Effects run again, in case what they do has changed.

A save with a mistake in it leaves the old code running, with the error showing on top until the next save.

## Writing one that stays fast

`tests/apps/tasks.tsx` is a small real app. Rendering costs what renders, so two
habits matter once a list gets long, and both of them are in that app:

- **Keep state as far down as it can go.** What's being typed in a box belongs in a component of its own. Kept in the
  component holding the list, each key renders every row.
- **Give rows what only changes when they do.** A row skips rendering when what it's given does the same as before (see "Not
  rendering what would come out the same"), so `onClick={() => pick(index)}` is fine, as long as `pick` doesn't see something
  that changes each time, as the selection. A handler that sees the selection is different whenever that is, and every row
  renders. Read it from a ref, or give the row the plain value instead, as `selected={index === selected}`. `memo` and
  `useCallback` aren't needed for this, but do no harm.

A long list of a page's elements, as rows of `<div>`s in something that scrolls, costs what changed and what's seen when the
selection moves or it scrolls, as far as `rsui/web` can tell: a list rendered again with the same rows, made anew as React makes
them, isn't looked through again for what kind of thing each row is (`keptKids` in `web.js`), `scrollIntoView()` finds only what's
around a row, and laying the list out again lays out only the rows that changed, with the scrollbar it had. What it costs to start,
and what it takes, still goes with how many rows there are, as each is styled and made. `content-visibility: auto` on the rows,
with `contain-intrinsic-size` for how big one is taken to be, has a row far from being seen made as a box of that size with
nothing in it, until it's within half the window of what shows of it, and then as it is, from then on. Each row is still an
element of the page, for `querySelectorAll()` and refs, and the list is as long as those sizes make it; what's in a row that
hasn't been near yet isn't there for code to find, but the text written in it is what its box is named, for agents and screen
readers. `content-visibility: hidden` is a box of that size until it stops saying so. `bench/long-list` measures a list of each
kind with no window: see its README.

## Seeing why it renders

Ctrl+shift+I in a window shows what renders as it happens. Each component that renders gets an outline where its output is, blue
if that changed something and yellow if it didn't, with its name and how many times it's rendered while that's showing. A meter in
the corner has the last 60 frames, each split into script, layout, paint and handing it to the GPU, with a line at the 16.7 ms there
is for one. Pointing at anything says which component made it, how often that's rendered in the last 30 seconds, and why it last did:

```
Row rendered 4 times in the last 30 s, 3 of them changing nothing.
Last: Row (0.1 ms, changed nothing): `onSelect` is a new function each time. App makes it at app.tsx:38. useCallback would keep the same one.
```

The why is in sentences, from what rsui knows rather than what it guesses. Which props aren't the same, and why not: a function made
again in the same place each time, an object or array with the same things in it, or a value that changed from one thing to another.
Which state was set, by the name it has in the code, from what to what, and on which line. A context it reads changed. Hooks go by the
variable they're put in, so `const [count, setCount] = useState(0)` is `count`, and one inside a hook of the app's own is
`useTodos → items`, which is read from the line that calls it.

With `--do`, `why T` says the same about what made `T`, along with what it's in and what each of its hooks holds, and `frames` tells
each frame since it was last asked as a story: what came in, which handler ran and where it's written, what rendered and why, which of
that changed nothing, and where the time went. `--agent` has `ui_why` and `ui_frames` for the same. Both keep this from the start,
so there's nothing to turn on before something goes wrong, and `inspect off` stops it. `inspect show` draws it in what `shot` takes.

Every change of state is kept as well, numbered, with what it was before and after and where it was set. `history` lists them,
and `goto 3` puts every component's state back to how it was just after the third, or forward again, with `goto 0` before them
all. What happens next carries on from there, as after an undo. Only state goes back: refs, files and what's been sent stay as
they are.

`logpoint app.tsx:40 "items.length"` says what that is each time line 40 runs, without stopping anything, along with what the app
logs. It's put in front of the line as the file's read, so it sees what's in scope there before the line runs, and the app takes
in the code with it as it does a saved file, keeping its state. Only a line that starts a statement can have one, which is checked
before anything runs. `logpoints` lists them with how often each has run, and `logpoint off app.tsx:40` or `logpoint off all`
takes them away. `--agent` has `ui_history`, `ui_goto` and `ui_logpoint` for these.

`break app.tsx:40` is a breakpoint. With no window there's nobody to say when to carry on, so each time line 40 runs it says what
the variables there have in them, and carries straight on; in the viewer (below) the app stops there. The first one has the app's
own files, those in its folder but not in `node_modules`, read again with somewhere to stop in front of each statement, and the
app takes in that code, keeping its state. Nothing else is read again, and until then nothing's any slower: the engine only has
these in a file it's been told is being debugged, where each is a load of one flag until there's something to stop for, and the
JIT leaves a function with them to the interpreter. `breakpoints` lists them, and `break off app.tsx:40` or `break off all` takes
them away.

**Replay.** Everything that comes in to the app is kept, as a window gives it: the pointer, the keyboard, typing, scrolling,
resizing, dropping, and when it came by the app's clock, apart from what's typed into a password, which is kept as `*`s. So is when its JavaScript timers went off, which is a little after
they're due, as the app gets to them, and where `Math.random` started. A trace has all that in `replay`, and `rsui-tsx --replay
app.rsuitrace app.tsx` runs the app again from the start, giving it everything at the times it came, with `Date.now()` saying what
it said then and each timer going off when it went off. It does the same again, apart from what it read from elsewhere: a file,
the network, or what another thread worked out. `--to 12` stops replaying after input 12, and from there it goes on as usual, with
its clock going on from where it got to; `--pause` stops at the first statement of the app's own that runs for input 12, to go
through what it did a statement at a time, as many times as it takes. The viewer's Replay tab does that for any input.

`profile` sums up what's kept: the slowest frames, the components that took the most time of their own, and why renders that
changed nothing happened, most often first, which is usually where the time is to be won back. `rsui-tsx --profile 5s app.tsx`
runs an app for that long with no window and says the same, and `--agent` has `ui_profile`.

**A trace** is what's kept, saved to a file to look at later or attach to a bug: `save out.rsuitrace` with `--do`, or ctrl+shift+S
in a window while the inspector's showing, which saves it next to the app. `rsui-tsx --view out.rsuitrace` opens it in a viewer
that's an rsui app itself, with each frame's story, the components by time, every change of state, and the summary, so it shows
the same wherever it's opened. It's JSON, so anything else can read it too.

**While it runs**, `rsui-tsx --view` with no file looks at an app that's running in a window, in an inspector laid out as a
browser's is, with its tabs, dark or light as the system is. Each app run with
`rsui-tsx` listens on a named pipe for that, which only its own user can write to, and says it's there with a file in
`%TEMP%\rsui-live` that goes when it ends. The viewer finds it, or asks which if there are several, and `--view 1234` picks one by
its process. Listening is a thread that waits: the app keeps nothing until the viewer asks, and from then keeps what ctrl+shift+I
would.

- **Elements** is what's in its window as a tree of tags, with what the pointer's on filled in in the app's window. Beside the
  one chosen are its props, its box (margin, border, padding and what's inside), the component that made it with why that
  rendered and what its hooks hold, and what it listens for.
- **Console** is what `console.log` and logpoints say, and works out what's typed: at the top of the app, or while it's stopped,
  in the function chosen there.
- **Sources** is the app's own files. Clicking a line's number stops there, and right-clicking it adds a logpoint. "Pause the
  app" stops it at the next statement of its own that runs, even while it's too busy running its own code to get to anything
  else. Stopped, the line it's on is marked, and beside it are what's watched, the breakpoints, what's in the variables of the
  function chosen, and each function that's running; "Step over", "Step into" and "Step out" go on a statement at a time, and
  "Resume" to the next breakpoint. Its window doesn't draw while it's stopped, and it carries on by itself if the viewer goes.
- **Performance** is each frame as a bar, split into where its time went, and what came in, what rendered and why in the one
  chosen, with "Summary" for what `profile` says. **Rendering** is each component's time and how often it rendered for nothing.
- **State** is each change of state, with a button that goes back or forward to it, as `goto` does.
- **Replay** is what came in. "Replay to here" starts the app again in a window of its own, replaying it all up to that one, and
  "Replay and stop in it" stops in what that one did; the inspector goes over to the new one.

The three buttons at the top right keep the inspector in a window of its own, or inside the app's window along its bottom or
its right, as a browser's can be: the app's laid out in what's left, the strip between the two is dragged to share the room out
differently, and closing the app gives the inspector back as a window of its own. The eye next to them draws the flashes and
meter in the app's window, the next stops the lists moving while you read them,
and the last saves a trace next to the app. When the app stops, what was kept stays to be looked at. A packed app doesn't listen.

A request to the pipe and its answer are each a line of JSON, so anything else can ask too: see `src/live.rs` for what there is.

`RSUI_INSPECT=1` starts a window with it showing, which works for an app written in Rust too, and `RSUI_INSPECT=keep` keeps it without
showing it. While none of this is on, rendering checks one thing to find that out, and the layer one more for each hook, so an app that
isn't being looked at costs the same. A program made with `--pack` doesn't have ctrl+shift+I.

## What's different from React

- A component's output is one element, so an array or a fragment at the top of one is put in a `group`, which can change
  how it's laid out. Inside an element, fragments and arrays disappear as you'd expect.
- A component that renders nothing is still an element in the tree, one that takes up no room and that nothing can reach.
- Node's own modules are there as far as `rsui-js/DESIGN.md` says, which is most of what programs and packages use. `fs.readFile` gives
  a `Buffer` unless it's asked for text, as in Node, so `<text>{await fs.readFile(file)}</text>` needs `'utf8'` to show anything useful.
- `name` on an element is what agents and screen readers call it. If it has text inside it as well, that's what shows.
- There's no `ref` to an element, as `focus(key)` and `autoFocus` on `TextInput` do what they're mostly for. An error while rendering shows
  in place of that component, and the rest carries on, unless an error boundary says otherwise.
- A component given only plain values skips rendering when they're the same, with no `memo` needed. One the app's own JSX makes
  skips when what it's given does the same, functions and objects made again too, as with React Compiler. Mutating something
  inside a prop of a component that skips, and expecting that to show, won't work. So won't reading something that changed
  outside it, when its parent renders. `'use no memo'` in it says it renders whenever its parent does.
- On rsui's own elements, a click only reaches the one thing it's on, and what's around it doesn't hear about it, unless it says
  `e.passOn()` or what's around it has `onClickCapture`. A page's elements are as a page's are: see "Writing an app as for a page".
- `localStorage` is kept in `%APPDATA%\rsui-storage`, or the file `RSUI_STORAGE_FILE` says, in one JSON file for each program. The whole file is written
  on each change, so it's for settings, as it is in a browser, and not for a lot of data. `sessionStorage` is the same, for as long as the program runs.

### What a program can ask of the desktop

`rsui/desktop` has an object for each part of the desktop: `shell`, `clipboard`, `secrets`, `dialogs`, `notifications`, `tray`, `taskbar`, `shortcuts`, `sound`,
`app`, `protocols`, `fileAssociations`, `displays`, `system`, `power`, `network`, `drag`, `menus`, `storage` and `updates`. Each is only compiled the first time
it's used, so importing one loads nothing of the others. `Notification`, `Audio`, `alert`, `confirm`, `screen` and `localStorage` are there without importing
them, as they are on the web, as thin fronts over the same calls, in `rsui/web-routes`. `mainWindow` and `useCurrentWindow()`, from `rsui`, do what can be done
to a window, and say how it is. What the web and rsui both have a way into, as the title, storage and whether the window's shown, is one value underneath:
see `dom_state.js`.

All of it can be seen or heard or lasts, so it only happens in a program that's really running. With `--do`, `--agent`, `--png` and in tests, it's said instead, as these
steps let an agent check what the app does about it: `global KEYS`, `tray [ITEM]`, `menubar ITEM`, `popup ITEM`, `notification`, `soundends`, `secondinstance [ARGUMENT...]` and `power WHAT`.

How it's been checked:

- Really used, with nothing to see, on a desktop nobody can see: windows of each kind, seen from outside, the menu bar, a message box and its buttons, the second of a
  program, what a shortcut's message and a power message do, and the program ending. Sound was played from a silent file, to its end.
- Only by a person, as these show up on the real taskbar or last: tray icons and their menus, notifications, a real key press reaching a global shortcut, audible sound,
  dragging and the look of a window with no frame, and starting at login, where only the setting itself has been tried, in a key of its own.

Windows 11 puts a new tray icon behind the `^` next to the clock, until the person chooses to bring it out, and no program can do that for them. It holds back notifications while a game
or anything else is on the whole screen, and shows them in its list instead. A window's corners and shadow are the system's: a window with no frame gets both, and `radius` on `<window>` shows
nothing, as a window has nothing behind it to show in its corners.

What's different from Electron: notifications come from the tray's icon, or one that's there while they are, as they're the system's older kind, which needs nothing
installed. They can't have buttons. Message boxes only have the system's sets of buttons. A tray's menu and the menu bar are the system's own, so they don't go dark with the
app. `Command` in keys means Control.

### Code written for React

`react`, `react-dom`, `react-dom/client`, `react/jsx-runtime` and `react/jsx-dev-runtime` are all there to import, and packages get them too, instead of any
copy of React they've brought with them. These are in `src/react*.js`, and aren't compiled unless something asks for them. `Component`, `PureComponent` and error
boundaries work, along with `Suspense` and what React 19 has: `use`, `useActionState`, `useOptimistic`, react-dom's `useFormStatus`, `ref` as a prop,
a context as its own provider, and refs that give back what cleans them up. These have been run in an app and worked: Redux Toolkit, react-redux 7 and 9, redux-undo, zustand, jotai,
TanStack Query, swr, formik, react-hook-form's `Controller`, react-i18next, react-router 5's `MemoryRouter`, with `Link`, react-router 6 and 7's
`BrowserRouter` and `HashRouter`, with `Link`, `NavLink` and `useNavigate`, `@radix-ui/react-slot`, and for dragging, react-dnd with its HTML5 backend, @dnd-kit,
react-beautiful-dnd and @hello-pangea/dnd. That was by hand and not in the tests, which don't download anything.

What's different:

- Everything in the first list above about rendering when props change, which is as React with React Compiler has it for the
  app's own code. `memo` gives back what it's given, and so ignores a comparison of its own.
- `createPortal` shows what's in it where it's written.
- `use` waits for a promise by throwing it, as code written for `Suspense` does itself, and the `Suspense` nearest it shows its `fallback` until it's settled.
  What was in that `Suspense` starts again then, with new state, as there's nothing to keep it in while it's hidden, where React would keep what was already
  shown. With no `Suspense` around it, only what's waiting shows nothing. A `lazy` component waits this way while it loads.
- Nothing renders in the background, so a transition is shown as soon as it's done, as anything else is. One that's an async function lasts until it's settled,
  which is how long `useTransition` says it's pending, and how long what `useOptimistic` shows for it lasts. An action dispatched in one, or another
  transition started in it, keeps it going as long as that takes, as React has them entangled. `useDeferredValue` gives what it's given, or its
  `initialValue` the first time.
- `useFormStatus` reads the page's `<form>` around it, which is pending while a function given as its `action` runs (see "Writing an app as for a
  page"). Outside a form it's never pending.
- `preload`, `preconnect` and the like have nothing to start loading, as everything is here already. `preinit` would load a script or stylesheet
  and run it straight away, which can't be done, so it says so and does nothing.
- On rsui's own elements, a `ref` is never filled in, though on a class it's the instance, and on `forwardRef` or a function component given it as a
  prop, it's whatever that says. A page's elements (see below) fill one in.
- What's in a built-in element isn't in its `props.children`, so code that looks there, rather than at a component's children, sees nothing.
- On rsui's own elements, `onClick` gets an event that says where the pointer was, which button and what was held, and a click only reaches the one thing it's on,
  unless it calls `e.passOn()`, though `onClickCapture` around it hears it first, and `document` and `window` hear it unless `stopPropagation()` says otherwise.
  `onChange` is given the new value rather than an event, and so is `onFocus`. `onMouseEnter`, `onKeyDown` and the rest of the web's are there, given events.
  A page's elements have all of these as a page does.
- An error boundary catches errors while rendering. It doesn't catch those from effects.
- A promise that's rejected with nothing to catch it, once everything that was waiting has run, is logged with `console.error`, as `Uncaught (in
  promise)`, and the window hears `unhandledrejection` about it first, which can `preventDefault()` that. The app goes on, as a page does: only an
  error thrown while rendering, in an effect or in a handler shows over the app. The engine hands them to the layer for this (see
  `Engine::report_rejections_to` in rsui-js), and an engine nothing's asked that of still has them be an error.

### Writing an app as for a page

React written as it is for a browser works as it is, with nothing to import for it:

```tsx
import React, { useRef, useState } from 'react';
import './app.css';
import styles from './card.module.css';

export default function App() {
    const field = useRef<HTMLInputElement>(null);
    const [items, setItems] = useState<string[]>([]);
    return (
        <div className="app" onClick={(e) => e.target === e.currentTarget && field.current?.focus()}>
            <h1 className={styles.title}>Items</h1>
            <input ref={field} placeholder="Add one" onKeyDown={(e) => e.key === 'Enter' && setItems([...items, field.current!.value])} />
            <ul>{items.map((item) => <li key={item}>{item}</li>)}</ul>
        </div>
    );
}
```

A tag that only a page has, like `div`, `p`, `h1` or `ul`, is the page's element, drawn as CSS and a browser's own styles say, which is `rsui/web`'s. That's loaded the
first time one's used, so an app written with rsui's own elements pays nothing for it. `span`, `button` and `input` are rsui's own too, so they're a page's when they
have a `className` (or, for the last two, a `type`), or when they're in a file whose JSX is written for a page: one with a `className`, an `htmlFor` or a tag like
`div` in it. The engine finds that out as it reads the file, and only a file written that way and with one of those three in it is read twice, to call `__jsxPage`
rather than `__jsx` for its JSX.

On a page's elements:

- Events go from the window down to what they happened to, to `onClickCapture`, `onKeyDownCapture` and the rest of the `Capture` handlers on the way, and back up
  to the rest, unless `stopPropagation()` says otherwise. `document.addEventListener` and `window.addEventListener` hear them first on the way down, with
  `capture`, and last on the way back up, and that's for clicks, `mousedown`, `pointerdown` and the rest of the pointer, `keydown`, `keyup`, `wheel`, `paste`,
  `copy`, `cut`, composing, `contextmenu` and `dblclick`. A press on what doesn't take clicks is still a click there, so "click outside to close" works.
- Each says where the pointer was, from the window's top left in its pixels (`clientX`, `pageX`), from what it happened to (`offsetX`), and on the screen
  (`screenX`), and how far it's moved (`movementX`), which button and which are down, and what was held (`ctrlKey`, `shiftKey`, `altKey`, `metaKey` for the
  Windows key). Keys have the web's `key`, as `'A'` with shift, `'F5'` or `'Shift'`, a `code` from where the key is, so a shortcut doesn't move with the
  keyboard's language, `location`, `repeat`, `isComposing` and `keyCode`.
- `target` is what it happened to. That's what a `ref` holds, if it has one, so `ref.current.contains(e.target)` says whether a press was inside it, and
  otherwise one made up from the tree, with its class, its `data-` attributes as `dataset`, where it is, `closest()`, `matches()` and `parentElement`. What it's
  called is what it is in the tree, as `DIV` for a box and `SPAN` for text, unless it has a `ref`. Words are part of what they're in, as on a page.
- `onPointerMove` and `onMouseMove` are told at most once a frame, and only what has one is, so a page that doesn't listen pays nothing for the pointer moving.
  What the main button went down on hears it move until it comes up, wherever it goes, as `setPointerCapture()` would have it, so a slider made of boxes
  works. `onWheel` can stop what's under it scrolling. `onAuxClick` is the middle button, or back or forward.
- `onKeyDown` hears a key before what has the keyboard does anything with it, so `preventDefault()` keeps a field from typing a letter, or from selecting
  everything with ctrl+a. `onKeyUp` is there, and `onInput` is the same as `onChange` in a field.
- `onPaste`, `onCopy` and `onCut` have `clipboardData`, and `preventDefault()` stops them. `onCompositionStart`, `onCompositionUpdate` and `onCompositionEnd` say
  what's being put together by a way of typing, as for Chinese, Japanese and Korean.
- `onTransitionEnd` and `onAnimationEnd` say when CSS's transitions and animations finish. With nothing moving, as for an agent or a test, that's as soon as they
  start, so nothing that waits for one waits for ever. `onLoad` and `onError` on an `img` say whether it can be read, once what's running now has finished.
- What comes in and goes out is CSS's too. `@starting-style` (and Tailwind's `starting:`) says where an element that's just been drawn, or shown again after
  `display: none`, starts its transitions from; what's there from the first doesn't move. `transition-behavior: allow-discrete` (`transition-discrete`) with
  `display` keeps what's going to `display: none` shown until its transitions are done, and `visibility` always waits for its own. With nothing moving, it goes at once.
- `tabIndex` makes anything something tab reaches, in the order a page has it, and -1 something that can only be clicked to or given the keyboard with `focus()`.
- `draggable` is dragged once it's pressed and moved 4 pixels, which isn't a click then: `onDragStart`, `onDrag` and `onDragEnd` on it, and `onDragEnter`,
  `onDragOver`, `onDragLeave` and `onDrop` on what it's over, with `dataTransfer` for what `onDragStart` set. `preventDefault()` in `onDragOver` or `onDragEnter`
  says it can be dropped there, and escape stops it. Files, text and links dragged in from other programs come the same way, with `dataTransfer.files`,
  each with its `name` and its `path`, and `getData('text')`, which can be read all through it, not only once it's dropped. A window only asks the system to
  be told about those once something listens for a drop. Files that nothing on a page takes still go to `onDropFiles`.
- `onKeyDown`, `onMouseEnter`, `onMouseLeave`, `onPointerEnter` and the like, `onFocus` and `onBlur` are given events, and so is `onChange`, with `event.target.value`.
- A `ref` is filled in with something like an element: `focus()` works, and a field's `value` is what it shows now, which setting changes. A function
  given as a `ref` is given the element, and what it gives back, if that's a function, is what's called when the element goes or the `ref` changes,
  as in React 19. Otherwise it's given null then, as an object's `current` is set to.
- A `<form>` is sent by its buttons, by Enter in one of its fields, as its first button that sends it would, and by `requestSubmit()`. `onSubmit` hears
  it first, and unless that calls `preventDefault()`, a function given as its `action`, or as `formAction` on what sent it, is called with a
  `FormData` of what its fields say, as a transition, and `useFormStatus` in it is pending, with that data and `method`, until that's done. Then its
  fields that keep what's typed start again, as React has them. A field says what it gives the form while it's there: a box only when it's ticked,
  nothing when it's disabled, and the button that sent it its `name` and `value`. `new FormData(form)`, `form.elements`, `reset()`, a button with
  `type="reset"` and `submit()`, which sends it without `submit`, are there too. A string `action` has no page to go to, which is said.
- A field with no `value` keeps what's typed in it, however often what's around it renders, as `defaultValue` does in a browser.
- `import './app.css'` adds that CSS to the page, and `import styles from './x.module.css'` gives each class by its own name, as nothing can clash here.
  So does a `<style>`, or a `<link rel="stylesheet">` to a file of the app's, that's put in `document.head` with `appendChild`, `append`, `prepend`,
  `insertBefore` or `replaceChild`. `<head>` keeps its children in order, and the CSS in them counts in that order, with what's imported where a bundler
  puts its `<style>`, at the end of `<head>` when it's imported, so `prepend()` puts a sheet under it. It goes again when it's taken out, and changes
  with its `textContent`, or with `sheet.insertRule()` and `deleteRule()`, as CSS-in-JS libraries write it. A `<link>` says `load` or `error` once what's
  running now has finished. `document.head.querySelector()` and `document.querySelector()` find what's in it by tag and attributes.
- What scrolls has `scrollTop` and `scrollLeft`, which say how far it's scrolled as it was last laid out, or what code has just set, and setting them,
  `scrollTo()`, `scrollBy()` or `scrollIntoView()` moves it there, as far as it goes, straight away. `scrollHeight` and `clientHeight` are how big what's
  in it is and how much shows. It says `scroll` each time it moves, whether a person or code moved it, to `onScroll` and what's listening on it, and
  to what's listening on `window` and `document` with `capture`, as a page has it. The page's own scrolling is `window.scrollY`, `window.scrollTo()`
  and `document.documentElement.scrollTop`, and says `scroll` on `document` and `window`. Rust keeps where each is, in the scrolling area's state
  (`scroll_area_with` in rsui-runtime), which is told where code wants it as a number that's new each time, and tells the page when it's moved.
- A `<table>` is laid out as a table, which is a layout of its own in rsui-layout (`table.rs`): each column is as wide as what's widest in it in any
  row, wherever the rows come from, as a component that gives back a `<tr>` does, with what's left of a table that's said to be wider shared by how
  much each column wants. `colSpan`, `rowSpan`, `<thead>`, `<tbody>`, `<tfoot>`, `<caption>`, `<col>`'s widths, `border-collapse`, `border-spacing`,
  `table-layout: fixed` and `vertical-align` in a cell are there, and so is `display: table`, `table-row`, `table-cell` and the rest on anything. What's
  where a row or a cell should be and isn't one is in a row or a cell of its own, as CSS makes one up around it.
- A `<dialog>` is there while it's closed, for a `ref` to it, or `getElementById`, to find, and is drawn while it's open, as its `open` attribute
  says, or `show()`. `showModal()` puts it on top of everything, over its `::backdrop`, in the middle of the window: it's rsui's own dialog then, which
  has the keyboard to itself and gives it back afterwards, and pressing what's behind it does nothing. Escape is its `cancel`, which closes it unless
  `onCancel` calls `preventDefault()`. `close(value)` closes it with that as its `returnValue`, and `onClose` hears about it once what's running now
  has finished. A `<form method="dialog">` closes the dialog it's in when it's sent, with what the button that sent it says. `:modal`, `:open` and
  `[open]` are there in CSS.
- `window.history` and `window.location` are kept in memory, starting at `app://rsui/`, as there's no address bar: `pushState`, `replaceState`,
  `back`, `forward`, `go`, `state` and `length`, and `href`, `pathname`, `search`, `hash`, `assign` and `replace`, with `popstate` and then `hashchange`
  as a browser has them, so react-router's `BrowserRouter` and `HashRouter` work as they are. A link goes where its `href` says when it's clicked,
  unless its `onClick` calls `preventDefault()`, as `Link` and `NavLink` do: an address somewhere else is opened by the system, and one of the page,
  which can't load another page as there's only this one, is gone to in this one, with `popstate`, for a router to show it. Going nowhere, as
  `location.reload()` and `history.go(0)` would, does nothing.
- What a `ref` holds, and what an event happened to, is an element as a page has one: an `HTMLElement`, or the kind for its tag, as `instanceof`
  checks, with `nodeType` and `ownerDocument`. `addEventListener` on it hears what happens to it and what's in it, on the way down and back up, before
  the page's own handlers, as a browser has it where React's root is around them all, and `stopPropagation()` there keeps it from those.
  `setAttribute()` and `removeAttribute()` change what it's drawn with, as `draggable` does. `getBoundingClientRect()`, `offsetWidth` and the rest,
  and `parentElement`, are as it was last laid out. `document.elementFromPoint()` says what's at a place, `querySelector()`, `querySelectorAll()`,
  `matches()`, `closest()`, `getElementById()`, `getElementsByTagName()` and `getElementsByClassName()` look through the tree as it is now with the
  same matcher CSS uses, so they take what CSS does, kinds of element, `+`, `~`, `:nth-child(2 of .x)`, `:not()`, `:is()`, `:has()`, `:scope`,
  `:checked` and the rest, and `getComputedStyle()` says what its CSS came to: margins, padding, borders, `display`, `position` and how big it is. That's what
  drag and drop libraries ask of a page. Something only just made, as an effect can ask about, hasn't been laid out yet, so it isn't anywhere, and
  what's been laid out is where it was then, as a browser would have it only if nothing had changed. Rust keeps that, in `rsui::geometry`, which only
  holds on to the tree that was laid out while the next one's being made once code has an element of a page in a `ref`, as that makes changing it
  copy what it changes.

What isn't there yet: Touch events aren't there, as there's no touch
screen to have them from. A form's `elements` has its fields but not its buttons. `scrollbar-color` is only what something that scrolls says itself,
and the page's is what `html` or `body` says: it doesn't go down to what's inside, as it does on a page. `:nth-last-child(... of S)` only works in
`querySelector()`, as what comes after something isn't known as it's drawn. Scrolling from code goes straight there, whatever `behavior` says. `onMouseOver` and `onMouseOut` are told as `onMouseEnter` and `onMouseLeave` are, so they don't hear about what's inside. Something
dragged can't be taken out of the app to another program, or from one of its windows to another, and nothing's drawn for it while it's dragged.

### Code written for a browser

Code written for a page loads `rsui/web` the first time it uses a page's tag or CSS, and nothing else pays for it:

- `rsui/web` is what a page's tags and CSS are drawn with. CSS is worked out into props when something's drawn, and kept for as long as what it came to is the same. Variables, `rem` as `html` has it, `calc()`, classes that turn on a theme, `::before`, `::after`, what's said about things inside something, including inside its components, and fonts going down are all there. CSS-in-JS comes from its own packages: styled-components, emotion and the rest work as they are, with their rules read as they're inserted. There's enough of a `document` and `window` for code that touches them to keep going, and classes on `body` do what CSS says.

What CSS can't do here is said once when the app runs, starting with `rsui/web:`. Tried on 2026-11-10 with CSS of the kinds apps use, that was `backdrop-filter`, masks, `mix-blend-mode`, `writing-mode`, `font-variant-numeric` and `font-feature-settings` other than `normal`, `word-spacing`, `object-position`, `hyphens`, scroll snapping and `@container`. Logical properties, like `margin-inline` and `inline-size`, are their sides in text that goes left to right.

A sheet's rules count in the order a browser has them: what's in `@layer`s first, a layer at a time in the order the layers were first named (`@layer a, b;` names them), then what isn't in one. `@supports (name: value)` holds if `rsui/web` can do that without leaving anything out, with `not`, `and`, `or` and `selector()` as they are, so a check for what only Safari or Firefox has is false here. `@media` has `(min-width: ...)` and `(width >= 40rem)` alike. A rule inside another, as CSS nesting writes it, is a rule of its own, and what's under a condition inside a rule is part of it when the condition holds. A name with `\:` or `\[` in it, as tools write classes like `hover:bg-red-500`, is the name it says. `:is()`, `:where()` and `:not()` take whole selectors, which are checked against what something's inside and the page, so `:is(.dark *)` holds when `html` has `dark`. `+` and `~` are about what comes before something, `:nth-child(an+b of S)` counts only what `S` says, and something a step of a rule says has to be hovered, focused, pressed or ticked, as `.group:hover .x` and `.peer:checked ~ .x` do, keeps track of that, so Tailwind's `group-hover:` and `peer-checked:` follow it: something before others says how it is to what it's in, which draws them again (see `PEER` in `web.js`). In CSS, `:has()` looks at what an element's been given, and in `querySelector()` at what's in it in the tree, and `:host` is nothing, as there's no shadow tree. `scrollbar-width` and `scrollbar-color` draw a browser's own scrollbar in those colors, 11 across with `thin` and none with `none`, which still scrolls, and go over what `::-webkit-scrollbar` says, as Chromium has it. `svh`, `dvh` and the like are the window, as nothing comes and goes around it.

Variables are the page's, from `:root` and from classes on `html` and `body`, and also each element's own: what the rules on an element set, as `.shadow-sm { --shadow: ...; box-shadow: var(--shadow) }` does, is what it uses, and what's inside it has it too, unless `@property` says it doesn't go down. `@property`'s `initial-value` is what a variable is when nothing sets it, and so is what `*` sets. What uses a variable an element can set is worked out with the page's when a rule's read, as everything else is, and again for an element that sets one, as it's drawn, so an element that doesn't pays nothing more. `translate`, `rotate` and `scale` are put together with `transform` as CSS has it.

Boxes are laid out as a browser lays them out, in `rsui-layout`, which knows nothing about CSS but has the same rules:

- `body` is a box of its own in the window, which lays out what's in it as it says, so `body { display: flex; justify-content: center; align-items: center }` puts the page in the middle. What's in it then is the page's root element, as `#root` is, a block of its own. What `html` and `body` say about text and their background is the window's, as before.
- A fragment, and anything CSS says is `display: contents`, has no box: what's in it is laid out as part of what it's in, so a fragment's children are a flex box's or a grid's own items, and nothing of its box is drawn. In the tree it's still there, as a group as big as what it's in, at its top left, which nothing finds at a point. That's `display="contents"` in rsui-layout, which takes what's in it out to lay out with the rest and puts it back. A component that gives back a fragment of rows in a `<tbody>`, or of cells in a `<tr>`, is the one place that's still a box of its own, a cell made up around it, as a table looks for those itself. A fragment written in what an element has, as `{items.map(...)}` with `<Fragment key>`s, was never anything but what's in it.
- Something placed by itself that doesn't say where along a way, as `position: absolute` with no `top` or `left`, is where CSS's static position is: in a flex box, where the flex box would put it if it were all that's in it, so an icon over another in a button that puts what's in it in the middle is in the middle too, and in a block, under what's before it.
- Asked how narrow it can be, as a flex item that shrinks is, a flex box is as narrow as what's in it can be, and something that scrolls across is as narrow as what's in it can be, as CSS has them. So a table in something that scrolls, in a `main` that's `w-full` beside a sidebar, shrinks to where its text wraps, rather than making the page wider than the window.
- `grid-column-start`, `grid-column-end`, `grid-row-start` and `grid-row-end` go over that side of `grid-column` and `grid-row`, wherever each is said, as Tailwind's `col-span-2 col-start-2` has them. Each side can be a line, from the end too, `span` and a number, or `auto`, and `span 2 / 4` ends at line 4.
- A picture's `width` and `height` can be any CSS size, as react-icons' `width="1em"` is, which is as big as the text it's in.
- `lh` is a line of the text it's in, and `rlh` one of the page's, which is 1.2 times the size of that text, what `line-height: normal` is near. It isn't what the element's own `line-height` says.

#### CSS that the app's tools make

Some CSS isn't what a browser's given: it's made from what's written by the app's bundler, with the app's PostCSS settings, and Tailwind is a compiler that makes the CSS for the classes the app's code uses. So `rsui/web` does what the bundler did, with the tools the app has installed, and nothing it has of its own: in `css_build.rs` and `css_build.js`. If the app has PostCSS settings, the CSS goes through them, read with the app's own `postcss-load-config` and with every plugin they list, which is how Tailwind 3 runs, `daisyui` and all. Otherwise, if it has Tailwind 4, that's run on it, as `@tailwindcss/vite` would, and CSS it has nothing to say about is left as it was. With neither, or with no `node_modules` at all, the CSS is used as it's written.

They're run in an engine of their own, on a thread of their own, so the app's engine never loads them, nothing they leave behind is the app's, and `--pack` doesn't take them. That's rsui's engine: nothing needs Node. A package's stylesheet, for `@import`, is what its `exports` say for `style`, found as `require` finds a module. Tailwind 4 is given everything that could be a class name in the app's code, which is read in Rust as Tailwind's own scanner reads it, without knowing what the text is: the app's files that have loaded, and what `@source` adds. What's in a package isn't the app's code unless `@source` says so. Tailwind 3 looks for classes itself, as its settings say.

What they make is kept in `node_modules/.cache/rsui`, one file for each piece of CSS, with what it was made from at its top: the files the tools read, as their sizes and times, and a fingerprint of the class names in the app's code. While those are the same it's used as it is, which is reading one file, so the tools only run again when the app's code has a class in it that wasn't there before, or a setting or a stylesheet changed. It's made once the app's code has loaded, as that's what has the classes, and again when more of it loads, or a file's saved and the app runs again. What went wrong isn't kept. A packed app has what was made when it was packed inside it, and only uses that.

Measured on 2026-11-10 in a debug build, with the converted tauri-react-starter (Tailwind 4 and shadcn) and gecko-desktop (Tailwind 3 and daisyUI, through PostCSS): making it the first time took 0.16 s and about 4 s. With it kept, it took 5 to 7 ms the first time it's asked for as an app starts, most of that reading the app's code for class names, and under a millisecond after that, as only a file that's changed is read again.

What isn't there yet: what an `@import` names that isn't installed is left out, and said once. The engine can't load `.node` addons, so `@tailwindcss/postcss` and `@tailwindcss/node`, which load `lightningcss`, can't run; Tailwind 4 is run as its own compiler instead, which is what those run. `rsui-migrate convert` keeps the configs these tools read, known by what they bring in, and the packages that those and the app's CSS bring in, so a fresh install has what they need.

## What it costs

Measured on 2026-11-06, on a Ryzen 7800X3D, in a release build.

One small app was written both ways and run with no window. It varied a lot with what else the machine was doing,
so take the ratios more seriously than the times. Each number here includes finding the element, as an agent would.

| What | Rust | TSX |
|---|---|---|
| Mounting, including starting the engine for TSX | 0.8 ms | 3.4 ms |
| One button changing | 30 µs | 33 µs |
| A parent and its 1,000 rows rendering | 88 µs | 304 µs |

In a real window the peak working set is 18.2 MB, against 18.1 MB for the gallery written in Rust, held there by a limit while
the GPU starts (see `PERFORMANCE.md` in `rsui-core`). Woken from idle it's 4.6 MB against 4.7 MB. The peak private bytes are
53.8 MB for both. About 50 MB of that is the same in a window with almost nothing in it, as it's what Direct3D and its driver
take, and it mostly isn't in RAM. The binary is 2.7 MB against 1.35 MB.

While nothing was happening, the TSX gallery used between 0 and 47 ms of CPU in each 4 seconds, with no pattern to it, and
nothing in the layer polls. The system's CPU clock only counts in steps of 15.6 ms, so that's a few ticks. The Rust gallery
showed none in one run and about 100 ms in each 4 seconds in another, so this is too noisy to say more than that neither is
doing much. The cause of the higher Rust run wasn't looked into.

A saved file showed on screen about 50 ms after it was written, 40 ms of which is waiting for the save to finish.

## How it's tested

`--do` is how an agent uses a running app, in one shell command, with plain text coming back. There's no server to set up, and no state
between runs, so each one starts the app again and what a list of steps does is always the same. Steps end at a new line or a `;`, and a line
starting with `//` says nothing.

| Step | What it does |
|---|---|
| `look` | Prints every window's tree |
| `click T [ctrl] [shift] [alt] [win]`, `rightclick T`, `middleclick T` | The pointer goes to the middle of it, scrolling there first as someone would, and presses, holding what's said |
| `hover T`, `move T` | The pointer goes to the middle of it, with nothing pressed |
| `clickat X Y [ctrl] [shift] [alt] [win]`, `moveat X Y` | The same, at a place in the window, in its pixels, for what has nothing to find it by |
| `downat X Y`, `upat X Y` | The main button going down, or coming up, at a place in the window, for a drag that takes as long as a hand's, with `moveat` and `wait` between, which code that waits for the next frame needs |
| `drag T FROM TO [STEPS]` | Across it, from and to a share of its width, with the main button down, in steps |
| `dragto T T2 [STEPS]` | From the middle of one to the middle of the other, with the main button down, in steps, which drags what's `draggable` |
| `dropfiles T PATH...`, `droptext T text` | As if dragged in from another program and dropped on it, which says if nothing there takes it |
| `type T text` | Puts the keyboard on it, and types, in place of what's there. `""` empties it |
| `write text` | Types, wherever the keyboard is |
| `compose text` | As a way of typing for Chinese, Japanese or Korean does it: it starts, shows what it's come to, ends, and that's typed |
| `press KEY [T]` | A key such as `enter`, `tab`, `a`, `f5`, `shift` or `ctrl+shift+s`, going down and coming up, with the keyboard on `T` if that's given. `alt` and `win` can be held too |
| `scroll T ROWS` | The wheel, over it |
| `act T ACTION [INPUT]` | Any action in its `[brackets]`, for what would take a lot of steps by hand, such as `act #draft set_value "hello"` |
| `expect "text"`, `expect no "text"` | Fails if that's not so, right now, or after a moment if there's something going on |
| `wait MS`, `wait "text" [MS]`, `wait no "text" [MS]` | For what comes later: a step only waits for what's happening now, and at most a second |
| `window ID`, `window "Title"` | Which window the next steps are for |
| `close` | The window's close button, which may ask first |
| `dialog "path"`, `dialog cancel` | What the next file or folder dialog gets. They never come up, and with nothing said they're cancelled |
| `size W H` | Makes the window this big |
| `animations on`, `animations off` | Whether transitions, CSS animations and smooth scrolling play, from that step on, in every window. They're off unless a step says, so each run and its shots come out the same, and a run can settle the app first and then turn them on to watch one. CSS that asks `prefers-reduced-motion` follows. With them off, the first time something would have moved, stderr says once that `animations on` would play it |
| `shot file.png` | A picture of the window, drawn on the software GPU |
| `why T` | Why what made `T` last rendered, how often it has, what it's in, and what its hooks hold, by their names |
| `frames` | Each frame since this was last asked, as what came in, which handler ran, what rendered and why, and how long each part took |
| `profile` | The slowest frames, the components that took the most time, and why renders that changed nothing happened |
| `save F` | Saves what's kept as a trace, for `rsui-tsx --view F` |
| `history` | Every change of state, numbered, with what it was before and after and where it was set |
| `goto N` | Puts every component's state back to how it was just after change `N`, or forward again. `goto 0` is before them all |
| `logpoint F:L "E"` | Says what `E` is each time line `L` of file `F` runs, without stopping it. `logpoint off F:L` or `off all` takes it away, and `logpoints` lists them |
| `break F:L` | Says what the variables are each time line `L` of file `F` runs, as a breakpoint the viewer stops at. `break off F:L` or `off all` takes it away, and `breakpoints` lists them |
| `inspect on`, `off`, `show`, `hide` | Keeps that or stops, and draws it in what `shot` takes or doesn't. It's on to begin with |

`T` is `#key`, `@path`, or `"text"` that it's called. All of them are in what `look` prints. Presses, moves and keys go through everything a window's do,
with where they are, so what a page's code hears about them, `clientX`, `ctrlKey` and the rest, is what it would hear from a person. What a person couldn't do fails the same way: a
disabled button, or the main window while a modal one is open. Something covered up isn't refused, as the press lands on what's on top, and
the step says that nothing changed. Links only say that they were asked for. Copying and pasting stay inside the run.

`--agent` is the same app for a client to keep open, over the Model Context Protocol on the standard streams. It suits working on an app for a
while, where `--do` suits checking something and being done, and they're built from the same parts.

| Tool | What it does |
|---|---|
| `ui_snapshot` | Reads a window's tree, with the version, and which windows there are if there's more than one |
| `ui_find` | Looks through every window for what matches, unless it's told which |
| `ui_act` | Carries out an action in a node's `[brackets]`, on what has this `key`, or is at this `path`, and says what changed, leaving out what only changes how something looks. Given the version that was read, it's refused if things have moved on. `set_value` always gives the app text, as typing would |
| `ui_wait` | Waits for the version to move on, with timers counting too, which an action doesn't wait for |
| `ui_screenshot` | A picture of a window, as an image in the result |
| `ui_key` | A key such as `ctrl+s`, with the keyboard on a node if that's given, by `on_key` or `path` |
| `ui_close` | The window's close button, which may ask first. Closing the main one ends the app until `ui_restart` |
| `ui_restart` | Starts again from nothing |
| `ui_why` | Why what made a node last rendered, how often it has, and what its hooks hold, by their names |
| `ui_logpoint` | Says what an expression is each time a line runs, in the logs, or takes that away |
| `ui_history` | Every change of state, numbered, with what it was before and after and where it was set |
| `ui_goto` | Puts every component's state back to just after a change in `ui_history`, or forward again |
| `ui_profile` | The slowest frames, the components that took the most time, and why renders that changed nothing happened |
| `ui_frames` | Each frame since this was last asked: what came in, which handler ran, what rendered and why, and how long it took |
| `ui_next_dialog` | What the next file or folder dialog gets, as they never come up. With nothing said they're cancelled |

Each file the app has loaded is looked at when a tool's called, so a save counts by the next call however soon that is, with no waiting to be told
about it. An app that won't start doesn't end the server: each tool says why until a fix is saved. What the app logs comes back at the end of a tool's
result, in quotes as text from the app is. Nothing it writes can reach the client any other way, as the standard output goes to the standard error
as soon as this starts, for anything it might run as well. What a person couldn't do is refused here too. `ui_act` runs a node's action rather than
pressing where it is, so something covered up isn't refused, which `--do` is the one for.

`tests/app.rs` runs real TSX files with no window, and checks them through the tree an agent would see: hooks and when they
run, context, keys, timers and promises, inputs, popups, long lists, scrolling, errors, garbage collection under load, and
saving new versions, including ones with mistakes in them. `tests/web.rs` does the same for code written for a page, events
included: a slider made of boxes dragged by where the pointer is, ctrl and shift clicks, a menu that closes when something
outside it is pressed, a letter stopped before a field types it, and the rest. `tests/apps/gallery.tsx` has the same tree and draws the same picture as the gallery that was written in Rust.
