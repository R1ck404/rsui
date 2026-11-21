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

**The platform's look:** `native` on a `button`, `input`, `checkbox`, `switch`, `slider`, `progress` or `window` draws it as
the platform's own: Material 3 on Android and Fluent on Windows (and on Linux, for now), or what `theme` in `rsui.config.ts`
forces, with `RSUI_THEME` over that. It's rsui's element, styled by `rsui::native_look` with props any element has, so it costs
what one styled by hand does (`bench/native-look`: a thousand of either made again in 6.6 ms). What the app says of how it looks,
`className` included, is left out, and said once in a dev build; where it goes stays. `android={{ variant: 'tonal' }}` and
`windows={{ variant: 'accent' }}` are read for that look only. What changes as it's used is drawn from props too: Material's ripple
(`ripple`, kept by `App` from where it was pressed), a field's line while it's typed in (`focus_line`), and the platform's focus
ring (`focus_ring`). Dark or light follows the system, and native controls are made again when it changes.

```tsx
<button native onClick={save}>Save</button>
<input native value={name} onChange={setName} placeholder="Name" />
<switch native checked={on} onChange={setOn}>Wi-Fi</switch>
```

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
  themselves. What refers to itself, as an object of handlers that each call the others through it, is the same when everything
  else in it is: a pair already being compared further up is taken to be the same again inside. It looks at no more than 1,024
  objects and 16 levels down, and an array of 64 at most, so it costs a fraction of a render, and it never runs any of the app's
  code. The engine can do this because it knows each closure's code and what it captured, one value or cell for each variable it
  uses, and nothing else.
- **Handlers that see something new.** A handler, a prop named as `onClick` is (`on` and a capital), made from the same code
  as the one the child has, is the same even when what it sees has changed, as `() => run('refresh')` does when `run` sees the
  selection: the child's handler is handed what the new one sees (its captures are set to the new one's, in place,
  `Vm::props_equivalent_handing_on`), so that it's the same function doing what the new one would, as `useEvent` or a ref holding
  the latest does by hand. What the child drew and its effects call that, and an effect that depends on the handler doesn't run
  again, as it wouldn't for `useEvent`. That's only done once everything else in the props is the same, and only for a function
  written in the app's own files, which isn't async or a generator (one of those can be waiting part of the way through), and
  whose own properties nobody has asked for. A function named anything else, as `renderRow` or `pane`, is handed on in the
  same way as long as every call of it while something rendered had what it read noted, and no hook keeps it as a
  dependency, as what called it is looked at again then: see "What a component read" below.
- **Where.** The parser makes JSX call `__jsxMemo` (layer.js), which marks a component's element, and the compiler has it call
  `__jsx` instead in a file that isn't the app's own, as one in `node_modules`, so a library's elements are as React has them
  (TanStack Table's `flexRender` gives cells the same `row` object, which says something else once it's selected). Only a marked
  element is compared this way, as React Compiler only keeps what it compiled, and only for a component written in the app's own
  files (each compiled function knows, in its proto's `says`): a library's components, and rsui's own page elements, which read
  the page's CSS as they render, render as they would in React. An element `__jsx` made says so (`memo: NOT_KEPT`), and is
  only the same as itself wherever it's found, as a cell made by `createElement` inside the app's `<div>`.
- **The page's elements.** A plain tag the app's own JSX wrote, as `<div>` or `<button>`, is compared the same way once it's a
  page's element (`__rsuiEquivalent` in web.js, which `webDescription` in layer.js and the place each is told keep marked), as
  React Compiler keeps a `<div>` whose children it kept: a `<div>` given children made again that do the same isn't drawn again,
  and nor is what's in it. One already wasn't when given the same objects. Only while the page's CSS is as it was when the old
  one was made, though (each says which version it was made under, `__pageMark`), as a sheet that changes is only told to some
  of what's drawn and the rest relied on being drawn again by what it's in. And what's in its children is compared as cautiously
  as a component's props are, so that a component in it given the same ref or object with methods still renders. A field, an
  `<option>` or a page element a library made renders as before. What a page element passes down (its place among the others, for `:nth-child()` and `a + b`) is the
  same as before when what's before it only has handlers or refs made again, or a `style` holding the same, as those are never
  attributes (`sameToCss`), so what's after it isn't drawn again either. What a page element puts around each thing in it to
  tell it something, where it is among the rest or that it's on a line (`wrappedIn` in web.js), is compared in the same way, so
  it doesn't render again for being made again around what does the same, and nor does what's in it. A picture's shapes are
  given the same function for what CSS says of them while the rules and variables it works from are the same, and a picture is
  only drawn again for a variable that it or its shapes use.
- **Being careful.** The same object is the same, as React's `memo` and React Compiler have it, as React's rules say what's given
  isn't changed in place, but not when it holds a function or `current`: a ref changes without a render, and an object with
  methods, as TanStack Table's `row`, can say something else when they're called, and a component can do either as it renders.
  (`memo` takes those to be the same, as React's does.) What a handler sees is only read when it's called, so it's the same as
  itself there. A class renders whenever its parent does. What reads a context or a store (`useSyncExternalStore`, and the
  libraries made on it) is told to render by the provider or the store, whatever its props are, and state still renders what
  it's in. What isn't covered is a component that reads something else that changes as it renders, as a variable outside it, or
  calls a function it's given that reads one, which React Compiler gets wrong the same way. Nor is a component that calls a
  handler (`onSomething`) while it renders, and shows what it gives, or code that keeps an old handler to call later expecting
  what it saw then: React's rules say a handler isn't called while rendering, and the old one now does what the new one does.
- **Saying no.** `'use no memo'` (or `'use no forget'`), as React Compiler has it, at the top of a file or of a function's body:
  the JSX there calls `__jsx`, and a component that says it renders whenever what it's in does (`Vm::says_no_memo`).

Why not compile it, as React Compiler does: that's working out, for each value in a component, what it's made from, and getting
that wrong shows something old, where comparing can only cost a render. Comparing also works for what React Compiler can't
keep, as elements made in a loop or in `renderRow`. What the parent works out as it renders is kept in its own way: see "What a
render works out" below. A component that says `memo` compares in the same way, as it's
asking for the same thing, but without leaving out refs and objects with methods; given what wasn't the app's own JSX, `memo` is
as React's, comparing each prop as `Object.is` does.

What it comes to, moving the selection with the Down key in `demos/file-explorer` on a folder of 5,000 files, headless, with
each key's frames added up (the median of 40 keys, the middle of 6 runs of each). "Before" is comparing alone, before what refers to itself,
handlers handed on and the page's elements were:

| | Components rendered | Frame time |
|---|---|---|
| The demo as written, with `memo`, `useCallback` and its own `useStable` | 24 before, 12 now | 3.0 ms before, 1.8 ms now |
| A copy with all of those taken out | 109 before, 14 now | 10.2 ms before, 2.0 ms now |

What was left in the copy without them was the app's object of handlers that call each other through it, which ran out of
levels going round; the bar's buttons, whose handlers see what changed; and the panels' `<div>`s, which rendered whenever the
panel did, and told what's in them where it is anew whenever a handler before it was made again. What's left now is a `<div>` a
handler made again is in that also holds what changed, which renders, as the hand-written one's does.

### What a component read

Comparing props whole renders a component whenever anything in them changed, even what it never looks at. The file explorer gave
each pane the whole remembered session, which is saved again whenever another pane gets the keyboard, and a pane only reads its
own folder's view and what's pinned: every pane rendered for every move of the keyboard. So the engine notes what each render
reads of its props, and when they're new and not the same, a component whose reads all come out the same again doesn't render
(`Vm::reads_same`, rsui-js `reads.rs`, which `JsProps::eq` asks after `Vm::props_equivalent_handing_on`).

- **What's noted.** In an app's own files, a function named with a capital that's given its props first, as React has a
  component, says when it's the one rendering (a Note at its start), and each read of its props, or of a name destructured
  from them (`function Pane({ session, id })` or `const { session } = props` at the top), is noted as the whole chain of
  property reads, as `session.views[id]`, with what came out at its end. That's compared when the props are new by the rules
  above, as a prop would be, so `session.views[id]` is the same while that folder's view is. Each render notes its own, as what
  a render reads depends on what it was given (`open ? a : b`). A read is one instruction, `Op::Note`, after the reads it's
  about, which does nothing but test a byte while nothing's rendering, in compiled code too, and steps over the keys after it.
- **Read whole.** What's read as itself is read whole: the props, or a name, given to an element or a call, spread, listed with
  `Object.keys` or `for in`, gone through with `.map`, or anything else that isn't a property read. A method called on what was
  read (`list.map(...)`, `pinned.includes(x)`) reads all of what it's called on, as it sees it as `this`, and a native goes
  through all of it. A getter, a proxy, a `Map` and the rest are read whole where the chain gets to them, as looking into them
  would run something, and then they're only the same as themselves. What's read whole and is the same object as before is the
  same, as what's read of it is noted for itself, except, for what didn't say `memo`, what a method was called on or a function
  read from (`careful`): TanStack Table's `row.getIsSelected` can say something else when it's called with nothing given anew,
  so a `row` with methods renders whenever its parent does, as before.
- **Read as itself.** What's compared with `===` or `==`, or given to a call, or put in the array given to a hook (a call of
  `use...`: its dependencies), has to be the same value, not one that does the same, as an effect that depends on something
  runs again when it's a new one in React. What's compared with `===` or `!==` and nothing else only says whether it was the
  same as the other side, as in `active={id === focus}`: that's what's noted, so moving the focus between two other panes leaves
  this one's the same. Only one side of a comparison is noted that way, as two such sides could each change into the other.
- **Seen later.** What the component makes as it renders that sees its props, or a name destructured from them, as a menu's
  handler that reads `session.pinned` when it's clicked, sees them through a cell (a component's props are always in one when
  something inside sees them), and when it skips rendering, the cell is given the new props' value: what the last render made
  then does what the new one's would, as handed-on handlers do. Read as the component renders, by a function it calls, what
  that function reads is noted for it, through the cell. The component's props are the new ones after it skips, so what it
  renders next for its own state reads them.
- **Functions called as something renders.** A function in the props, as the file explorer's `pane={drawPane}`, which sees which
  pane has the keyboard, is a new one each time the app renders, and everything that passes it down rendered for it. What it
  reads of what it captured is noted when it's called as something renders, through where it was read from (`pane`, then what
  it captured), and what it makes as it runs reads what it captured through it too. Then the component that called it doesn't
  render while what it read is the same. What only passes it down, as the splits and their parts do, hands it on instead, as a
  handler is: the old one is made to do what the new one does, and each render that called the old one is looked at again,
  with what it read gone over with the function's new captures, and renders again if any of it changed (`take_unheld_readers`).
  Handing on shares: the old function comes to hold the new one's block of captured variables (`Upvalues::share`, counted, so
  the last to hold it frees it), and so does whatever held the same block as the old one (`Vm::join_handed`), and two found to
  do the same are joined in the same way. That's what keeps a child that skipped, which holds the old function, following what
  it's in, which holds the new one, or, as a page element that kept its old children does, an older one still: whichever of
  them is handed on next, all of them are, and what read any of them is looked at again. Only one that's running keeps its
  block, and gets the new captures put in it instead. Two that already hold the same block see nothing new for being handed on
  again, as a page's button that kept its old handler is each time what drew it renders, so nothing's looked at again for them.
  And looked at again with the props it has, a render only asks whether what was handed on reads something else: an object it
  read a function from is the same as itself then, though it's only that when new props are compared (see `cautious`), or what
  read `onClick={props.model.add}` would render again for nothing, and again each time it did. Which closures share a block is
  kept by their numbers, which the collector doesn't trace, so it forgets each that's freed (`forget_freed_closures`), before its
  place can be another's or its page given back.
  That's only for a function every call of which, as something rendered, had its reads noted, which is known as a read of a
  captured variable that wasn't marks the function as one that can't be handed on (`NOT_HANDED_ON`), as can one made by such a
  function; and one that no hook keeps as a dependency (`keepAsItself` in layer.js), as an effect that depends on it would run
  again in React. What a function reads of the top of a module isn't noted: a component that reads something else that
  changes as it renders isn't covered, as before.
- **What it costs.** A Note that isn't for what's rendering, a component's own read when it isn't the one rendering, or a
  captured variable that isn't a destructured name's cell or seen through a function read from the props, is a test of a byte,
  and while something renders, a look in one map and a mark on the closure. A function that's marked is
  `READ_ONLY_LENGTH`'s bit, which only an array uses: the collector's `OLD` bit is on every kind of object.
- **What's still rendered.** A render whose frame wasn't found, as a component behind `forwardRef` is called by another function, or one that sees its `arguments`, has nothing noted, and is compared as above. `RSUI_READS=0` turns all of this off,
  for comparing.
- **Given to a function.** What a component reads and gives straight to a plain function of the app's, as `paint(look, selected)`
  or `cell(width, formatDate(entry.modified, look.dateFormat))`, is read where that function reads it: the function notes what it
  reads of its parameters through where each was read (`NOTE_PARAMS` in rsui-js `op.rs`), so `paint` reading `look.colors` and
  `look.zebra` is the row reading those, and a row that isn't selected doesn't depend on `look.selection`, which `paint` only reads
  for one that is. That's for a parameter that's a plain name nothing gives another value, of a function that isn't async or a
  generator and doesn't look at its `arguments`; one that a function inside it sees is read whole, as that one can read anything
  of it later. What's given to anything else, a native, a library's function, or one that's called some other way, is read as
  itself, as before: the call notes what it's given just before it's made (`NOTE_CALLING`), and what's called takes it at its
  start, or it's read as itself once it's clear nothing will.
- **Given on to a child.** What a component only gives on to elements its own JSX makes, as a prop, as a row gives its `look` to
  its `Icon`, is the same as long as what each component it went to read of it is: when the child renders, or is kept from
  rendering, inside what's rendering, what it read is linked to what gave it that prop (`Vm::link_child`), and when the parent is
  given new props, a prop of them it only gave on is checked against what each child read, with the new one in its place
  (`given_hold` in `reads.rs`). That's only when every place it was given is such a child, by the count of times it was given,
  and none of them read their props whole. If they all hold, each child's props are given the new value in place, as is what it
  made that sees them through a cell, so that what it renders next by itself sees what the parent would have given it. A child
  that renders again by itself keeps its link (`Vm::move_link`).
- **Looked up.** `has` on a `Set` or a `Map`, and `get` on a `Map`, called on what was read, as `selected.has(entry.path)`, is
  only what it gave: that's noted with what it was given (`NOTE_RESULT`, just after the call), and looked up again in the new
  value when the props are compared, which runs nothing of the app's. So moving the selection only renders the rows whose
  answer changed, rather than every row that looks in the set.
- **A long list's rows.** Each row of `<list>` notes what `renderRow` read as it drew that row, through the function, as a
  component's props are (`Vm::start_function_reads`), and when the list is given a new `renderRow`, a row whose reads are all the
  same in it isn't drawn again (`RowProps` in bridge.rs). A row has a place among the components for being told to draw again,
  and `renderRow` itself is never made to do what a new one does, as only the rows drawn inside what's rendering would hear about
  that: each row compares for itself.

What it comes to in `demos/file-explorer`, headless, with three panes split side by side and a file clicked in the first, which
doesn't have the keyboard (the frames the click draws, added up). "Before" is the commit this started from; "the old app" is a
copy that gives each pane the whole session again, as it did before it was given only what it uses:

| | Components rendered | Splits, parts and what draws each pane (`Inside`) | Panes |
|---|---|---|---|
| The old app, before | 371 | 2 splits, 6 parts, 8 | 6 |
| The old app, now | 255 | none, none, 3 | 4 |
| The app as it is, before | 363 | 2 splits, 6 parts, 8 | 3 |
| The app as it is, now | 252 | none, none, 2 | 3 |

What's left is rsui/web's own around each page element, and the rows of the pane that was clicked, which render as its
selection does. In the old app, the third pane's `Inside` renders too, as the function that draws it gives it the session,
which is another, and then the pane itself, whose `look` the app makes again each time it renders and the pane gives to a hook.
The Down key in a folder of 120 files took 1.84 ms a press before and 1.90 ms now (the median of 240 presses each, which move
by 5% between runs), and starting took the same.

What reading through functions, children, lookups and rows came to, in the same app, headless, with the inspector's count of
what rendered and how much of that changed nothing (the frames of each step added up). Before is 16efa49e:

| | Before | After |
|---|---|---|
| Three panes, a file clicked in the first, which doesn't have the keyboard | 257, 184 changing nothing | 52, 15 |
| Another file clicked in the same pane | 47, 36 | 14, 3 |
| Down, in a folder of 5,000 files | 47, 36 | 14, 3 |
| Empty space clicked in a pane that doesn't have the keyboard | 121, 54 | 48, 17 |
| Empty space clicked in the pane that has it | 21, 11 | 10, 5 |

What's left changing nothing is a page element told where it is among the rest anew when what's before it changed how it looks,
which `a + b` could ask about, and a few told something new by what they're in that comes out the same for them.

### A page's plain elements, drawn in place

Most of a page is `<div>`s, `<span>`s and `<p>`s that are only what CSS makes of them. Each was a component of its own (`Styled` in
web.js), with its hooks, effects, and what Rust keeps for a component, for what a browser keeps as an element and its style. Now
each of those the app's JSX makes, of the tags that do nothing more than that (`PLACED_TAGS`: not a field, link, button, form,
picture or table), is drawn in place: as a host element of the component whose output it's in, by Rust as it builds that output
(`placed_in_output` in bridge.rs, `placeElement` in web.js). What it comes to is worked out by the same `Styled` as before, and
given back as it was each time that output's built again, until something it depends on changes, as a browser styles an element
again only once what its style depends on has:

- what it's given, unless that does the same as before, as a component's props would (`samePageProps`, which is `JsProps::eq`'s
  comparison for a page's element), so a handler made again isn't a change;
- its own state, as hovered or pressed, which the handlers Rust tells it with set;
- what it read of what's above it: it reads contexts through what stands for it among the components (`newFrame` in layer.js),
  which is told when one changes, and says what it read of the parent's look and variables as a component does (`readsHere`);
- the page's CSS changing, as every element of the page hears.

What it keeps is its `live`, the one object every element of the page has (`PlacedLive`), and that stand-in, which what's in it is
under, for what that reads and for what it passes down. A component in it is that stand-in's child. It has no hooks of its own:
its state, what `useMemo` kept, and where it is among what's in it are kept on its `live`, and what it does once it's on the page,
and giving its ref its element, is done as the commit it's drawn in starts, and undone once it's not drawn again (`placedCommit`),
as React has a host element's ref. When something it depends on changes, its owner builds what it gave back again without calling
it, as when a component's state is set back to what it was, which works out again only the elements that changed, and gives Rust
what's the same as the same object. Nothing in what scrolls or a long list's kept rows is drawn in place, as those are built by
themselves later rather than as the owner's output; each of their page elements is a component as before, and their rows'
own elements are drawn in place in the rows.

A long list's row that skips what's in it while it's far from being seen keeps that as it was given, rather than copied to give
each of its elements a key by where it is, and what's passed down to what's in an element drawn in place is only worked out once
something's drawn in it.

What code makes, with `document.createElement()` and the like, is drawn by a component of its own for each node already
(`OwnElement` in dom.js, and `drawNative`), so it's drawn as before.

It's on unless `RSUI_PLACED=0`, or the page says `globalThis.__rsuiPlaced = false`, as `tests/placed.rs` does, drawing each page both
ways and comparing what's drawn and what the page's code finds out.

What it came to, measured on 2026-11-18, headless, release builds (`lto=false`), the old and new one after the other on a machine
other builds were using. In `bench/long-list` (rows of Tailwind `<div>`s, each in the app's own `Row` with a ref and a handler):

| Rows | | Before | After |
|---|---|---|---|
| 1,000 | starting | 774-908 ms | 680-1,015 ms |
| 1,000 | memory to start | 63-66 MB | 51 MB |
| 5,000 | starting | 793-834 ms | 657-658 ms |
| 5,000 | memory to start | 95 MB | 73 MB |
| 25,000 | starting | 3,545-4,062 ms | 2,883-2,896 ms |
| 25,000 | memory to start | 418-423 MB | 305 MB |
| 25,000 | memory after the steps | 568-579 MB | 490 MB |
| 25,000 | Down, median of 21 a run, five runs | 13-24 ms | 10.5-20 ms |
| 25,000 | Down, scrolling, median of 40 a run, five runs | 20-33 ms | 17-32 ms |

The key presses moved between two levels from run to run on both, as others were building, so they're no slower that can be
told. What a row still costs to start is mostly working its look out, as each row's element is still styled as it's first drawn,
and the app's own `Row` and what it makes. In `demos/file-explorer`, with three panes (the inspector's count of what rendered and
how much of that changed nothing, as above):

| | Before | After |
|---|---|---|
| A file clicked in a pane that doesn't have the keyboard | 52, 15 | 40, 12 |
| Another file in the same pane | 14, 3 | 10, 2 |
| Down, in a folder of 5,000 files | 14, 3 | 10, 2 |
| Empty space clicked in the pane that has the keyboard | 14, 4 | 7, 1 |
| Empty space clicked in a pane that doesn't have it | 54, 23 | 39, 18 |

None of what's left changing nothing is a page element told where it is among the rest anew: drawn in place, one given the same
as before isn't worked out again. It's the app's own components that read a context that changed, providers given new children,
and the app's state set to what renders nothing new. The explorer's first frame and memory to start were the same (about 70 ms of
script on this busy machine, both, and 25.6 MB private against 25.7).

A child is the same component as last time if it's in the same place, as in React: where it is in the array of children it's in, with `null`, `false` and `undefined` keeping their places, and the same for each array and element around it. An array or a fragment is one place, with its own places inside. A key takes the place of where it is in its array. A component of another type in a place, or one inside an element of another type, is a new one, and what was there is unmounted.

### What a render works out

Skipping a component whose reads are the same saves its whole render, but a component that does render works out everything in
it again. The file explorer's pane sorts a folder of 25,000 entries and makes a map of where each one is, and it renders whenever
the selection moves. So a plain call in a component's or a hook's own body, as `const items = shownItems(folder, listing, ...)`,
gives what it gave at the last render when it's given the same and nothing it was worked out from has changed, and an app writes
the calculation plainly (rsui-js `keeping.rs`).

- **Where.** The compiler puts a Note of `NOTE_KEEP` before each call of a function by name whose answer is used, in the body of
  a component or a hook (`useSomething`) of the app's own files, and one of `NOTE_KEPT` after it. A hook is never one of these
  calls, as React counts them, and nor is anything in a function inside the body, as a handler. Each place keeps what it gave
  for each component, by the host's number for it, and for each time it runs in that render, so a loop in the body keeps one for
  each turn. While nothing renders, each Note is a test of a byte, as the others are.
- **Only what's worth it.** A place is timed the usual way, by the two Notes, and is only kept from once its last call took 50 µs
  or made 64 KB, not counting its first, which compiles what it calls, so a quick call pays two clock reads and nothing more.
  The first time a place is found to be worth it, the call is made the usual way; from the next, the Note makes it, and keeps
  what it gave. In compiled code, a call given again or made by the Note is jumped over, so the rest of the render stays
  compiled.
- **What it's worked out from.** Everything that can be reached from the function and its arguments: what's in them, what's in
  that, through prototypes, and through what each function reached captured, so the variables at the top of a module that a
  function reads, and what those hold, are part of it, as `scale` in `return list.map((x) => x * scale)` is. Each object of it
  is watched through the collector's write barrier: an old object goes the slow way when it's first changed anyway, and a young
  one that's watched is given the old flag too, which is taken off before each collection and put back after it, so the
  collector never sees it (`Watch` in gc.rs). Changing a watched object, by anything at all, stops it being watched and counts
  a change. Each cell a function reached captures is kept with what it held. The result is watched in the same way.
- **Given again** when the function is the same one, or one made from the same code that captured the same, its arguments are
  each the same value or the same by the rules above (a props object made again with the same in it is), nothing watched has
  changed since, and every cell kept holds what it did. Any change forgets everything kept, as it's one count for all of it.
  What's watched has been looked through and hasn't changed, so working out the next call only looks through what's new: the
  listing a sort was given is watched already when the query changes.
- **What isn't kept.** A call that changed something it didn't make (a watched object or a kept cell, as `calls++` or
  `cache.set(...)` on a module's own variables, or an argument changed in place), called something that isn't pure (the host's
  functions, the program's that runs the engine, `Math.random`, `Date.now`, `new Date()`, `performance.now`, promises, `WeakRef`:
  each native says, `NativeData::pure`), threw, or reached what can't be watched: a typed array's bytes, which change without the
  barrier, a promise, a generator, an iterator, a proxy, what's held weakly, a function sharing its captures with a handler handed
  on, or more than 400,000 objects. What a call like that did happens again at every render, as it would. Nor is one whose
  arguments took longer to look through than the call took, as one that reads a little of something big and new each time. A
  place that isn't kept three times in a row is made the usual way for sixteen calls before it's tried again.
- **What it costs.** Watching a folder of 25,000 entries the first time looks through about 25,000 objects, a fraction of what
  sorting it takes, and two bits for each object the engine has. At most 16 results are kept, the least recently given going
  first, and none for more than 30 seconds without being given again, as nothing says when the component it's for has gone:
  each holds on to what it was given and what it gave until then.
- **What isn't covered.** What's reached only through the global object, which is watched itself, so a global set again is seen,
  but isn't looked into, as everything is in it; and the language's own prototypes and what's on the global object as the
  engine starts, as `Array.prototype` and `Math`, which aren't watched or looked into, as the engine changes some of them itself
  (a method written in JavaScript is put in place the first time it's called) and a program doesn't.
  And a caller that changes what it was given back, in place, after it was given it again: the next render works it out anew,
  but in the render that changed it, the object is the one the last render had too, so a child given it then is given the same
  object as before and can skip, where in React it would have a new one. React's rules say not to change what's rendered with.
  `RSUI_KEEP=0` turns this off, for comparing, and `RSUI_KEEP_LOG=1` writes a line for each call worked out, each given again,
  and each time what was kept is forgotten, with what changed.

What it came to in `demos/file-explorer`, headless, on a folder of 25,040 entries, against the commit before, whose pane had
`useMemo` for its order and its map of where each entry is (the median of 8 runs of each, alternating, on a machine others were
using too, so only differences bigger than a few percent mean anything). With 40 presses of Down, the pane's order was worked
out once and given again 40 times, and so was the map.

| | `useMemo` | Plain calls, kept |
|---|---|---|
| Down, each press | 2.63 ms (best run 2.16) | 2.30 ms (best run 2.25) |
| Typing in the filter, each key, sorting again each time | 23.5 ms | 20.7 ms |
| Opening the folder, until its last frame | 459 ms (best 388) | 493 ms (best 382) |
| Starting in it, until its last frame | 12.8 ms | 12.4 ms |
| Peak private memory, starting / pressing Down | 58.9 / 61.1 MB | 61.6 / 61.1 MB |

Watching the folder the first time looked through 25,166 objects. Typing looks through only what's new, as the listing is
watched already.

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
  rendering what would come out the same"), so `onClick={() => pick(index)}` is fine, even when `pick` sees the selection, as a
  handler (`onSomething`) made from the same code is handed what the new one sees. A function named otherwise, as `isPicked`,
  that sees the selection is different whenever that is, and every row renders: give the row the plain value instead, as
  `selected={index === selected}`. `memo` and `useCallback` aren't needed for this, but do no harm.

A long list of a page's elements, as rows of `<div>`s in something that scrolls, costs what changed and what's seen when the
selection moves or it scrolls, as far as `rsui/web` can tell: a list rendered again with the same rows, made anew as React makes
them, isn't looked through again for what kind of thing each row is (`keptKids` in `web.js`), `scrollIntoView()` finds only what's
around a row, and laying the list out again lays out only the rows that changed, with the scrollbar it had, scrolled to or not.
Each row is compared with what it was once each time the list renders again, in Rust where its props are the same (`propsSame`),
and what scrolls keeps the rows that are the same as they were rather than building them again (`KeptIn` in `bridge.rs`).

Rows far from being seen are skipped by themselves, as `content-visibility: auto` would skip them, where that changes nothing a
browser shows or says (`LongRun` in `web.js`): in a block with 2,000 or more things in it, which scrolls or is in what scrolls, as
the page does, and isn't edited, a row that's a flex box or a
grid with a height of its own, which is as big as it is whatever's in it, so the list is as long and each row where it would be,
and that has only the page's own elements and text in it, with no component, ref, field or link. Its elements are still the
page's, kept as they're written (`skippedParts`), so `querySelectorAll()`, `textContent`, `innerHTML` and the rest find them in
their place. What a browser does differently is lay one out the moment code asks where it is or how it looks; here that has its
row drawn from the next frame on, and until then it's nowhere, as what isn't shown is. `scrollIntoView()` on one brings its row
into view. A row that's near being seen is drawn as it is, and stays drawn. `RSUI_AUTO_SKIP=0`, or `globalThis.__rsuiAutoSkip =
false`, turns it off. What it costs to start, and what it takes, still goes with how many rows there are, as each row itself is
styled and made, but no longer with what's in each. `content-visibility: auto` on the rows,
with `contain-intrinsic-size` for how big one is taken to be, has a row far from being seen made as a box of that size with
nothing in it, until it's within half the window of what shows of it, and then as it is, from then on. Each row is still an
element of the page, for `querySelectorAll()` and refs, and the list is as long as those sizes make it; what's in a row that
hasn't been near yet isn't there for code to find, but the text written in it is what its box is named, for agents and screen
readers. `content-visibility: hidden` is a box of that size until it stops saying so. `bench/long-list` measures a list of each
kind with no window: see its README.

What each of a page's elements costs is kept down as a browser keeps it down, by sharing what's the same. An element's handle,
as a ref or `querySelector()` gives it, has only what's its own (its key, what it's of, what it says about itself), and gets
everything an element does from one prototype for each tag; its `style` is made the first time code asks for it. A long list's
rows of the same tag, classes and state, under the same things above them, start from the first one's look, as a browser shares one
computed style between such siblings (`sharedLook` in `web.js`), where nothing on the page asks about `:has()`, `inherit` or where
things are among what's around them, and the rules that reach a row are only about it alone, as Tailwind's are; anything else
works its own out. `RSUI_SHARED_LOOKS=0`, or `globalThis.__rsuiSharedLooks = false`, turns that off. What's the same about how
text is written is one object for everything that writes it so, and a row that skips what's in it passes nothing down to what's in
it until it's drawn. What's still each row's own is its component, with its hooks, the host element Rust keeps for it, and its
look as what it's drawn with: see the bench's README for what that comes to.

## Seeing why it renders

**The inspector is always there** in an app that's being worked on, which is any app run with `rsui-tsx` that isn't a packed
one. F12, or ctrl+shift+I, opens it docked along the bottom of the app's window, in the viewer described below, and pressing it
again puts it away. The first window also has a small pill in its bottom right corner: how long the last frame took, with a dot
that's green while frames take less than half the 16.7 ms there is, yellow up to all of it and red over, the last 24 frames as
bars, and how many frames a second there are while something's moving. Clicking it opens the inspector too, and its × puts it
away until the app starts again. It's only drawn when the app draws anyway, so it never costs a frame of its own, and
`RSUI_PILL=0` leaves it out. A packed program has none of this: no F12, no pill, no pipe and nothing kept. An app written in Rust
has it in a debug build, or with `rsui::allow_inspector(true)`.

**Where the time goes is kept from the start.** Every app being worked on keeps, lightly, the last 30 seconds of timed spans
(`rsui-spans`): each frame, and in it what came in, each handler, timer and lot of promise jobs, reconciling, layout, paint, handing
it to the GPU, waiting for the screen, and each garbage collection, a dozen or so a frame, so a stutter that's already happened is
there when the inspector opens. While the inspector's looking, everything is kept: each component's render, effects, invalidating
layout, telling geometry, shaping text and each call into JavaScript. `RSUI_SPANS=0` keeps nothing; with no window, as with
`--do`, nothing's kept unless `RSUI_SPANS=light` or `detail` says. While nothing's kept, each place a span could start loads one
byte.

**Recording JavaScript** samples where it is about every millisecond: another thread stops the app's thread, copies the top of
its list of running functions out of memory, and lets it go, so the engine checks for nothing while it's off and does nothing
extra while it's on. Each sample is the stack of functions, each with its file and line, outermost first.

Ctrl+shift+F draws what renders over the app as it happens, without the inspector. Each component that renders gets an outline where its output is, blue
if that changed something and yellow if it didn't, with its name and how many times it's rendered while that's showing. A meter in
the corner has the last 60 frames, each split into script, layout, paint and handing it to the GPU, with a line at the 16.7 ms there
is for one. Pointing at anything says which component made it, how often that's rendered in the last 30 seconds, and why it last did:

```
Row rendered 4 times in the last 30 s, 3 of them changing nothing.
Last: Row (0.1 ms, changed nothing): `isPicked` is a new function each time. App makes it at app.tsx:38. It sees something that changed. If it's only called when something happens, naming it as a handler (`onSomething`) lets it count as the same.
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
its process. Listening is a thread that waits: the app keeps only the light spans until the viewer asks, and from then keeps
everything, as F12 would. F12 starts this viewer itself, with `RSUI_VIEW_DOCK` set so it docks itself along the app's bottom.

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
- **Performance** is where the time went, as a browser's profiler shows it. Across the top is every frame kept, as bars as tall
  as the slowest frame in each part of the time, green, yellow or red; dragging across them picks a range, and clicking moves
  there. Under that is a flame chart of the range: the frames, then the main thread's spans as blocks inside what they were inside
  (a render that changed nothing is yellow), then, while JavaScript's being recorded, each function it was in. The wheel zooms
  where the pointer is, dragging moves along, and following what's newest stops once a range is picked. Clicking a block says what
  it was, how long it took and how long itself, and for a render why it happened; a function goes to its line in Sources.
  Below are **Summary** (how many frames, how many slow, and where the time went by kind), **Bottom-up** (everything by the time
  it took itself, spans and sampled functions together), **Treemap** (the components, or the functions, each as big as the time it
  took itself), **Renders** (the busiest frame's renders, with why each happened) and, with "As text", what `profile` says.
  "Heat in app" draws over the app's window how hot each element is over the range: by render time, by how many renders, or by
  renders that changed nothing, from blue to red, with the hottest named. The record button starts and stops sampling
  JavaScript. **Rendering** is each component's time and how often it rendered for nothing.
- **State** is each change of state, with a button that goes back or forward to it, as `goto` does.
- **Replay** is what came in. "Replay to here" starts the app again in a window of its own, replaying it all up to that one, and
  "Replay and stop in it" stops in what that one did; the inspector goes over to the new one.

The three buttons at the top right keep the inspector in a window of its own, or inside the app's window along its bottom or
its right, as a browser's can be: the app's laid out in what's left, the strip between the two is dragged to share the room out
differently, and closing the app gives the inspector back as a window of its own. The eye next to them draws the flashes and
meter in the app's window, the next stops the lists moving while you read them,
and the last saves a trace next to the app. When the app stops, what was kept stays to be looked at. A packed app doesn't listen.

A request to the pipe and its answer are each a line of JSON, so anything else can ask too: see `src/live.rs` for what there is.

`RSUI_INSPECT=1` starts a window with what ctrl+shift+F draws showing, which works for an app written in Rust too, and
`RSUI_INSPECT=keep` keeps it without showing it. While no viewer's looking, rendering checks one thing to find that out, and the layer
one more for each hook, so an app that isn't being looked at costs the same, apart from the light spans above. A trace saved since
version 2 has the spans in it too, so its Performance tab is the same as it was live. A program made with `--pack` has none of this.

Sampling sees compiled JavaScript as one frame, without what it called, so `rsui/web`'s own parts are timed by hand while
working on it: `profMark(n)`, a host function, adds the time since the last mark to mark `n` (`0` only starts again), and
`profMarks()` says what each has had. They're put in a copy of web.js and taken out after, as a call costs a little even
when nothing reads it, and `RSUI_DEV_WEB_JS=<file>` runs that copy in place of the web.js that's built in, so trying a change
to it needs no build.

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
  skips when what it's given does the same, functions and objects made again too, as with React Compiler, and a handler
  (`onSomething`) made from the same code is handed what the new one sees rather than rendering it. Mutating something
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

What a rule's declarations come to is kept for as long as what it read is the same, rather than worked out again each time
anything about the page's CSS may have changed. Working a sheet's rules out, or a rule about one class or one kind of element,
notes what it read that can change (`readsStill` in web.js): each page variable's value, whether each variable a `var()` names
is one an element can set, how big a rem is, and whether anything in it, or a variable it read, is in the window's units, as
`vh` is. While all of that is the same, and nothing's said that anything at all may have changed, as `@media` changing with the
window's size does, it's what it was, the same objects, so what was found from them is too: the page's rules' index, and each
element's plan, which says which of the page's rules could be about something with its classes, kind and id, and in which
order (`planOf`). A plan checks that the lists of rules about its classes and its kind are the same lists, with the same rules,
rather than being made again whenever the page's CSS changes (`planStill`). So a window made bigger, which only changes what's
in `vh`, works out the rules that use it and the elements they're about, rather than every sheet and every plan, and so does a
modal that sets a variable on `body` that the page's own sheet doesn't read. `RSUI_RESTYLE_CHECK=1` also works out again what's
kept this way, and says if it's different.

Measured on 2026-11-19, headless, `fastrel` builds (release without LTO), the commit before and this one alternately, seven
rounds each, the median of each step's median a run:

| | Before | After |
|---|---|---|
| `demos/file-explorer` on `C:\Windows\System32`, the window resized | 5.24 ms | 3.41 ms |
| the same, Down | 1.59 ms | 1.56 ms |
| Mantine's page, a `Modal` opened the first time, closed, opened again | 130, 107, 112 ms | 94, 80, 95 ms |
| shadcn's page, a `Dialog` opened the first time, closed, opened again | 197, 135, 144 ms | 174, 121, 146 ms |
| `bench/long-list`, 5,000 rows, Down | 6.9 ms | 7.1 ms, within the runs' spread |
| Private memory at the end: explorer, Mantine, shadcn, long list | 40.0, 153.2, 135.0, 87.4 MB | 39.4, 158.1, 136.2, 90.5 MB |

What's kept is held for longer, as every plan is now kept while it's what it would be made as, rather than only those made
since the page's CSS last changed: Mantine's page ends 3% bigger. A plan is one array of numbers for its items, and a count of
how often its rules were made again, to keep that small.

A plan is put together in Rust (`plans.rs`, `madePlanInRust` in web.js). web.js still finds the rules that could be about the
element, as they're found by its tag, id and classes, and the rules about its classes and kind, which finding works out; what it
hands over is numbers, six for each: where the rule is, what it says, how much, its sheet's place, its place in its sheet, and
whether it's about the element alone. What a rule says, the props it sets and which of them are `!important`, is a number Rust
gives the first time it's told them, the same for the same props, kept on the rule as its `weight` is (`saidIdOf`). Rust puts
the items in the cascade's order and finds whether what's about the element alone can go on all together (`itemsApart`), which
read every rule's props again for every plan: a Mantine modal made 95 plans each time it opened, as the page's list of rules is
another once react-remove-scroll adds its `<style>`, and those were about 170 µs each (they're taken from the index before now:
see `keptPlan` below). `RSUI_JS_STYLED=1` makes plans in web.js
as before, and `RSUI_CASCADE_CHECK=1` makes each both ways and says where they differ (`rsui cascade check: plan for ...`);
the explorer, Mantine's page and shadcn's had none.

What it costs to call Rust from web.js was measured first (`cssProbe` in layout.rs, called in each element of the explorer
with a copy of web.js put in with `RSUI_DEV_WEB_JS`, timed with `profMark`): a call that does nothing takes 0.06 µs, reading the
element's tag, classes and every prop 0.6 µs, as a JavaScript function reading them does, and making an object of 12 numbers to
give back 1.2 µs more (32 numbers, 2.2-4.8 µs more). What's slow is what's done, not getting there.

Measured on 2026-11-19 as above, plan making alone (marked in web.js, the same exe with and without `RSUI_JS_STYLED=1`): shadcn's
page starting 117-125 ms in web.js and 49-51 ms with Rust (1,303 plans), its first `Dialog` open 8.9 and 3.9 ms (145 plans). The
commit before and this one, seven rounds:

| | Before | After |
|---|---|---|
| Mantine's page, a `Modal` opened the first time, closed, opened again | 98.2, 80.9, 93.3 ms | 85.5, 72.5, 81.8 ms |
| shadcn's page, a `Dialog` opened the first time, closed, opened again | 176.8, 117.1, 145.1 ms | 184.1, 113.0, 122.8 ms |
| Mantine's and shadcn's pages started and closed (wall time) | 2,455 and 1,954 ms | 2,368 and 1,938 ms |
| `demos/file-explorer` on System32, Down and a resize; `bench/long-list`, 5,000 rows, Down | 1.61, 3.47, 4.63 ms | 1.60, 3.49, 4.55 ms |

shadcn's first open and its 40 keys typed (144 to 159 ms, each run's median key the same, 3.0 ms) are where a full collection
lands: a shadcn session has 17 full ones and 63 small ones with Rust's plans, and 15 and 68 without, with as much time in them
all (204-212 ms and 208-215 ms), and one of the 17 comes while it's typed in, for 14 ms. Over eight lengths of the environment
(`RSUI_PAD`, which moves where collections land) the keys came to 147-158 ms with Rust's plans and 135-143 ms without.

Private memory at the end, the median over eight lengths of the environment: Mantine's page 155.5 and 152.5 MB, shadcn's 133.1
and 137.9 MB, the explorer 39.6 and 40.9 MB, the long list 220.6 and 220.3 MB. shadcn's 4.8 MB more is there with
`RSUI_JS_STYLED=1` against without in the same exe, and is the process heap's: 3 MB more committed with as much allocated (38.7
and 41.7 MB committed, 31.5 and 31.3 MB allocated), and 1.2 MB more of the engine's objects, after 73 collections rather than 83.
What Rust keeps is about 100 KB: what 1,207 of shadcn's rules say, 4,408 props in all.

Whether each of a plan's rules about more than the element itself is about it is found in Rust too (`matching.rs`,
`matchedInRust` in web.js). Mantine has some sixty of those for every element, nearly none about it: rules whose last step is
`*:where(h1, h2, ...)`, as its typography's `.m_d08caa0 :where(h1)` is, are found by `*`, so every element looks at them, and
`ruleMatches` took about 0.6 µs each, 37 µs an element. Each rule's selector is read into Rust once, from the steps
`readSelector` made, and the number it's kept by is on the rule; a plan keeps those of its rules, with which states each is
about. For an element, one call says of each of them whether it's about it, about it only in a state it isn't in
(`stateNeeds`), or for web.js to work out; what they say is put on in web.js, in order, as before. Rust matches each step as
`stepMatches` does, and remembers what it found for each place above an element as `matchedAbove` does, by a number kept on that
place, for as long as `version` and `flowLearnedCount` are the same. `+` and `~` go on from what's before the element among
what it's in, which web.js gives each element it describes (`prevs`), with the same chain above, as `matchChain` and `matchFrom`
do (since 8174ab30; below). It doesn't read a selector with `:has()`, validity, `:nth-child(... of S)`, `@container` or
`:scope`, and says to work out `:focus-visible` on what's focused without a ring and an attribute whose value is an object,
which web.js matches as it did. `RSUI_JS_STYLED=1` matches in web.js, and
`RSUI_CASCADE_CHECK=1` matches each both ways and says where they differ (`rsui cascade check: match for ...`); the explorer,
Mantine's page and shadcn's had none.

On Mantine's page, the same exe with and without `RSUI_JS_STYLED=1`, all of `fromAbove` (plans and matching): the first modal
open 36.2 and 18.3 ms, 20 keys typed 14.9 and 7.9 ms, 20 turns of the wheel 45.9 and 23.3 ms, starting 580 and 330 ms. The Rust
call is about 10 µs an element, most of it in long `:where()` lists, each of which is looked at one selector at a time.

The commit before plans were made in Rust and this one, seven rounds:

| | Before | After |
|---|---|---|
| Mantine's page, a `Modal` opened the first time, closed, opened again | 97.0, 82.6, 92.0 ms | 78.8, 64.6, 75.2 ms |
| Mantine's page, 40 keys typed in a field; a turn of the wheel | 321, 19.1 ms | 296, 16.3 ms |
| shadcn's page, a `Dialog` opened the first time, closed, opened again | 175.9, 117.7, 143.4 ms | 165.1, 115.0, 125.5 ms |
| Mantine's and shadcn's pages started and closed (wall time) | 2,464 and 1,951 ms | 2,206 and 1,935 ms |
| `demos/file-explorer` on System32, Down and a resize; `bench/long-list`, 5,000 rows, Down | 1.62, 3.52, 4.65 ms | 1.61, 3.41, 4.69 ms |

shadcn's 40 keys are where a full collection lands, as above (146 and 160 ms). An `expect` step, which looks for text in the
whole tree, came to 0.7 ms more on both pages (5.5 and 6.2 ms on Mantine's), which plans made in Rust alone didn't, and which
isn't explained yet: the tree it looks in is the same. Private memory at the end, the median over four lengths of the
environment: Mantine's page 155.7 and 154.8 MB, shadcn's 133.1 and 135.7 MB, the explorer 39.7 and 41.1 MB.

Most of those sixty rules couldn't be about the element whatever is above it: `*:where(h1, h2, …)` is only ever about an `h1`
to an `h6`. What a step needs of an element's tag, id and classes, and what's last in at least one selector of each `:is()` and
`:where()` it has, is the same for every element a plan is for, as a plan is made for one list of classes, kind of element and
id (`planOf`). So once for each plan, `cssMatchPlan` marks each rule whose last step couldn't be about it (`Store::could_be`),
and `cssMatchItems` says no to those without looking. The same exe, with and without it (a copy of web.js put in with
`RSUI_DEV_WEB_JS`), five alternating rounds on Mantine's page: a turn of the wheel 16.5 -> 15.7 ms, the `Modal` opened again
72.2 -> 71.1 ms, 40 keys the same (295 ms). `RSUI_CASCADE_CHECK=1` had no differences on Mantine's page or shadcn's.

Two exes of the same JavaScript can differ by more than that: the commit before this one, against this one, seven rounds, had
Mantine's 40 keys at 302.6 and 274.7 ms, and an `expect` step at 6.0 and 5.5 ms, while each exe gave the same with either
web.js put in (forty `expect`s in a row on a page that's still: 4.6 ms in one exe whichever web.js, 4.0 in the other, 3.9 in
the one before plans were made in Rust). An `expect` restyles nothing, and looks for its text in Rust; the 0.7 ms an `expect`
came to more since rules about more than an element were matched in Rust was the exe, not what it does. So a change to web.js
is measured in one exe, with and without it, and a change to Rust over the session as a whole.

Why shadcn's 40 keys have a full collection in them now (looked at on 2026-11-19, with each collection placed in the step it
came in, over eight lengths of the environment): plans and matching in Rust make less garbage, not more (57 small collections in
a shadcn session rather than 68, as much made old in all or less), so more of the small collections keep most of what was made
since the one before, and each of those makes the next full one come early (`Generations::full_due`, once a quarter of the old
budget has become old). As shadcn's page starts, five of its full collections came that way rather than two, which moves the
rest along: one lands in the keys on every length of the environment (147-160 ms), where before it did on one of eight
(136-158 ms). Those early ones freed 19-46% of what small collections had made old since the last full one. Making them wait
for half the old budget once the last full one found most of that alive gave 13 full collections and the keys at 131-138 ms,
but `bench/long-list` ended 8 MB bigger in private memory (228.5 against 220.3 MB, the same working set: room the list of
objects had kept, on empty pages), and the explorer 0.8 MB, so it wasn't kept. A window collects when it's quiet after
starting, which `--do`'s `wait` doesn't, and which would put the collections somewhere else again.

shadcn's 2-4 MB more at the end is room, not what's kept: the small blocks' chunks are 2.6 MB more with 3.4 MB less of them in
use (57.5 and 40.0 MB, against 54.9 and 43.4), and the strings' list 1 MB more with more of it free, after a higher peak of
short-lived objects between collections that are fewer (73 rather than 83). Giving the engine's empty pages back after each
full collection (`RSUI_GIVE_BACK=1`) or the allocator's empty chunks (`idle`) still left 2-3 MB of it, in chunks that are
only partly used. What Rust keeps for plans and matching is about 100 KB.

An element's own variables are worked out in Rust (`variables.rs`, `withElementVariables` in web.js): what the rules on it set,
as Mantine's `--button-bg: var(--mantine-primary-color-filled)`, taken out of its look and worked out with what's around it,
what in its look waits for one of them declared again with them put in, and what goes down to what's inside it given back as a
`Map`, as web.js gave it. It reads and writes the look, the `Map`s and the note of what was read in the order web.js did, so what
comes out is the same props in the same order; it asks web.js what a page variable is (`variableValue`, which notes the read),
which props a property sets (`propertyKeys`), and, for two objects JSON alone can't tell apart, `sameValue`. What a declaration
came to with its variables put in is kept in Rust, as `resolvedDeclarations` kept it. A Mantine element has about ten
variables of its own and five declarations that wait for one, and web.js took 60-90 µs of an element's 300 on key presses for
them, in building strings, regular expressions and copying maps. Timed with `profMark` in one exe, two copies of web.js with
and without it: on Mantine's page 62-65 -> 18-22 µs an element on key presses, 69-78 -> 21-26 µs as its modal opens, 35-42 ->
13 µs as it starts; on shadcn's 37-50 -> 16-18 µs on key presses and 10-12 -> 6-9 µs as its dialog opens. `RSUI_JS_STYLED=1` works
them out in web.js, and `RSUI_CASCADE_CHECK=1` works them out both ways, on a copy of the look, and says where they differ
(`rsui cascade check: variables for ...`); the explorer, Mantine's page and shadcn's had none.

Rules found by an attribute, as shadcn's `[data-slot=...]` are, are matched in Rust too, as a plan's are (`extraMatchedInRust`).
That alone was no quicker: about half of shadcn's elements have such rules, and each of those took 70-80 µs in `indexedRulesOn`,
which was in putting every one of the plan's rules on one at a time, in order with the others, rather than all of what's about
the element alone together (see `orderWith` below).

Putting rules on, `mergeLook`, was tried in Rust first and was slower: with what `indexedRulesOn` found handed over to
`cssMergeLooks` all at once, shadcn's elements took 73-74 µs in `fromAbove` on key presses rather than 58-60, as a rule says a
few props, and web.js's `Object.assign` and its reads of a look keep where each prop is from one time to the next, where Rust
read and wrote each one through the engine. It's in Rust now (`looks.rs`, `cssMergeLook`, 313433ca), on what rsui-js gained for
it (crates/rsui-js/DESIGN.md, "Many properties at once"): the names it reads of a look and of a rule are a list found once and
read from the shape alone (`read_own_keys`), what waits for one of the element's own variables and is said again is found once
for each shape of look and taken out keeping a shape other looks share (`delete_keys`), and the props are copied with what goes
where remembered for each pair of shapes (`assign`). What's rarer (`!important`, a `display: flex` over a direction said before
it, a declaration whose props aren't known yet, one said again only in part) goes back to `mergeLookHere`. In one exe against
`RSUI_JS_STYLED=1`, seven rounds, merges an element: shadcn's key presses 34.4 -> 12.7 µs, Mantine's 8.0 -> 3.0. With all
of what's in Rust then (plans, matching, variables and this) against `RSUI_JS_STYLED=1`, `fromAbove` an element: shadcn's key
presses 83 -> 65 µs, its dialog opening 110 -> 72; Mantine's 77 -> 24 and 158 -> 55. `RSUI_CASCADE_CHECK=1`
merges a copy of the look in web.js as well, and compares them prop by prop and in order, the longhands of `transition` and
`animation` by what they hold (`rsui cascade check: merge of ...`).

The commit before variables were worked out in Rust (41416697) and the one after rules found by an attribute were matched there,
measured on 2026-11-19 as above, seven rounds each, with the machine busier than for the tables before:

| | Before | After |
|---|---|---|
| Mantine's page, a `Modal` opened the first time, closed, opened again | 91.1, 70.3, 86.0 ms | 86.5, 64.8, 76.8 ms |
| Mantine's page, 40 keys typed in a field; a turn of the wheel | 366, 22.6 ms | 367, 20.2 ms |
| shadcn's page, a `Dialog` opened the first time, closed, opened again | 187.3, 132.7, 157.7 ms | 201.5, 127.5, 139.9 ms |
| shadcn's page, 40 keys typed | 203 ms | 187 ms |
| `demos/file-explorer` on System32, Down and a resize; `bench/long-list`, 5,000 rows, Down | 1.57, 3.37, 6.87 ms | 1.54, 3.37, 6.79 ms |

shadcn's first open is where its collections land, as above: in one exe with and without variables in Rust, `fromAbove` alone
went from 64 to 143 µs an element from one run to the next as it opened, while what variables took went down in each.

Private memory at the end, the median over eight lengths of the environment: Mantine's page 152.1 and 151.4 MB, shadcn's 136.4
and 135.8 MB, the explorer 41.2 and 41.2 MB, and the long list 220.7 and 228.6 MB. The long list has no variables, and neither
change runs for it: its private working set is the same (216.0 and 215.3 MB), and the 8 MB is pages of objects left empty
(62,848 free places against 1,664) after a peak of short-lived ones, as it was when collections were made to land elsewhere
above. It stays with both changes turned off in a copy of web.js, and doesn't come with the commit before's web.js with a
comment or an unused function added, so it's where collections land with the new web.js, not what's kept.

After `mergeLook`, what an element's page rules came to was split up with `profMark` (one exe, a marked copy of web.js), and
what was left was moved where it measured faster:

- **Rules with `+` and `~`** (8174ab30). They were handed back to web.js, and on shadcn's page they were nearly all of what
  was: Tailwind's `space-x-*`/`space-y-*` (`> :not([hidden]) ~ :not([hidden])`) end on a step about anything, so every
  element asked about each of them. As shadcn's page starts, rules matched in web.js went from 43,285 to 400, taking 188.8 ms
  rather than 8.0, and `fromAbove` 421 -> 275 ms in all; its first dialog open 48.6 -> 25.4 ms, closing it 29.7 -> 19.6.
  What's left for web.js is `:has()` (shadcn's 400 as it starts, 8 ms; Mantine's 1,342, 8-30 ms, and 240 in 40 key presses,
  0.3-0.4 ms) and `:where()` lists with something in them Rust doesn't read (Mantine's 132): `:has()` asks about what an
  element's been given to have in it, React elements and components found later to make something, which only web.js has,
  for 2-3% of `fromAbove` as a page starts and less after. Validity, `:nth-child(… of S)`, `@container` and `:scope` aren't on
  Mantine's page, shadcn's or the explorer.
- **Plans kept when react-remove-scroll adds its `<style>`** (abd98cba). Any change to the page's rules made a new index, and
  plans are kept for an index, so as a dialog opened (react-remove-scroll puts four rules into the middle of the page's) and
  closed, every plan was made again: shadcn 145 and 116, Mantine 95 and 85, a third of what an element took then. A new index
  keeps the one before it, three back at most, and the keys of the rules that are different (`indexBefore`); a plan none of
  whose keys are among them is taken from it (`keptPlan`), with where its rules are among the page's and where their sheets
  are worked out again (a rule about one class keeps its sheet for that), and is looked at as any plan is (`planStill`).
  shadcn's second open makes no plan and takes 145. `fromAbove` an element: Mantine's modal opening 48.0 -> 42.1 µs, closing
  45.7 -> 31.4; shadcn's dialog opening 49.4 -> 35.0, closing 47.4 -> 43.4.
- **Only the rules about an element are gone through** (0f5add0e). `cssMatchItems` gives back which states all the rules it
  said of care about, put together as `ruleOnSaid` gives each (`said_needs`), and `indexedRulesOn` only goes on for a rule
  that's about the element or one for web.js to match. Mantine's key presses 22.6 -> 18.7 µs, shadcn's page starting 61.3 ->
  55.5. `RSUI_CASCADE_CHECK=1` goes through each rule as before and compares what Rust put together (`matchedNeedsCheck`).
- **What's about an element alone put on together, with rules found by an attribute** (6dd163a6). For each plan and list of
  such rules (which depends only on the names of what an element's given), where each goes among the plan's and whether each
  half's rules about the element alone can go on all together first, as `plan.apart` says for a plan alone, is worked out once
  (`orderWith`, `selfLasts`); what waits for a variable counts as saying what that sets. 1,667 of shadcn's 1,883 such elements
  put one half or both on together. That part of `fromAbove`, key presses 17.9 -> 11.5 µs, the page starting 17.5 -> 16.5; a
  focus is 16.1 -> 18.1, as new plans have what their rules say found the first time. `RSUI_CASCADE_CHECK=1` puts the rules on
  one at a time first, keeps that, puts the look back and compares (`togetherCheck`), as `passedOn` puts things on the look
  itself.
- **`Object.assign`** uses the pairs of shapes `assign` remembers, for a plain source and target (5e9434d8): see
  crates/rsui-js/DESIGN.md.

Each was measured in one exe, the web.js before against the one after (put in with `RSUI_DEV_WEB_JS`), seven rounds, with a
different length of the environment each round (`RSUI_PAD`), as shadcn's collections land in its first open and a different
web.js moves them. `RSUI_CASCADE_CHECK=1` had no differences on shadcn's page, Mantine's or the explorer after each.

What an element costs now, all of `Styled` split up with `profMark` (key presses, Mantine's / shadcn's, µs an element): to
`fromAbove` 11.6 / 14.8, `fromAbove` 18.6 / 26.9, its inline style 10.9 / 1.1, inheriting 5.1 / 22.1, its own variables
19.7 / 18.2, `dir` and its own font 7.3 / 9.2, its font and what goes down 21.0 / 24.1, what's in it 20.8 / 11.2, `withNth`
12.2 / 12.7, lines 12.0 / 10.6, host props and events 8.8 / 9.8, and the rest to the end 44.5 / 43.2: about 200 µs. What's
still in web.js there is mostly making the objects that are what it gives (props, closures for handlers, React elements),
which Rust makes no faster than JavaScript's own literals (bulk_speed: 12 numbers in an object 97 ns from Rust, 109 ns as a
literal). `withNth`, split the same way: finding each child's kind, the counts and how much is as it was 4.5 / 2.6 µs, each
child's `Place` 1.6 / 1.9, describing each (`describe`, which `+`, `~` and `:has()` read) 3.4 / 6.7, `cloneElement` or the
provider round it 2.0 / 2.3, the rest 4.5 / 3.5: the indices are a few nanoseconds of it, and the rest are objects Styled,
matching and React read, so it stays in web.js.

The native sampler (`RSUI_SAMPLE`, `--features sampling-profile`, crates/rsui-tsx/src/sampler.rs) ended a JIT build at once with
an access violation in ntdll: compiled JS has no unwind information, and the walk took the word on top of the stack for a
return address, as for a leaf, and gave `RtlVirtualUnwind` a context that pointed anywhere. It now checks every step against
the stack's bounds from the thread's TEB, finds a frame with no unwind information's return address above it (at `rsp + 40`
first in a compiled function's body, else the first word that's just after a call in committed code), and finds the
interpreter loop's frame pointer again where the JIT's `rbp` was left in it. It also stops the thread only to read its
registers and copy the stack it's using, and walks the copy with it going again, as finding unwind information can wait for a
lock the stopped thread holds (one start in eight hung). 99.8% of stacks reach the thread's start. The inspector's sampler
(`sampling.rs`) only copies the engine's list of frames with `ReadProcessMemory`, and ran with it.

The session as a whole (t3/css-rust-styled's head, 67f35351, and the commit before variables were worked out in Rust,
41416697, against this one), measured on 2026-11-19 as above, fastrel exes, seven rounds, medians of steps:

| | 67f35351 | 41416697 | This |
|---|---|---|---|
| Mantine's page, a `Modal` opened the first time, closed, opened again | 89.2, 67.8, 79.6 ms | 86.0, 71.8, 81.0 ms | 81.4, 63.9, 73.9 ms |
| Mantine's page, 40 keys typed in a field; a turn of the wheel | 388, 20.8 ms | 371, 17.6 ms | 398, 18.8 ms |
| shadcn's page, a `Dialog` opened the first time, closed, opened again | 235.6, 139.0, 160.9 ms | 202.3, 137.0, 160.0 ms | 165.5, 126.0, 153.1 ms |
| shadcn's page, 40 keys typed | 200 ms | 213 ms | 200 ms |
| shadcn's and Mantine's steps, all together | 917, 1,131 ms | 900, 1,030 ms | 848, 1,088 ms |
| `demos/file-explorer` on System32, Down and a resize; `bench/long-list`, 5,000 rows, Down | 1.56, 3.46, 6.08 ms | 1.59, 3.43, 6.07 ms | 1.55, 3.36, 6.44 ms |

Mantine's 40 keys typed range 315-436 ms in each exe, as its collections land; eleven more rounds of Mantine's page alone,
against 67f35351, had them 390.7 and 382.3 ms, its modal 84.1, 68.9, 81.0 ms against 79.4, 60.2, 73.7, and all its steps
1,121.6 against 1,090.0 ms. The long list's Down is 4.5-7.5 ms in each exe, so its 6.1 against 6.4 is within what one run of it moves by.

Private memory at the end, the median over eight lengths of the environment, 67f35351 and this: shadcn's page 136.1 and 137.3
MB, Mantine's 151.9 and 148.2 MB, the explorer 41.0 and 39.0 MB, the long list 228.6 and 228.8 MB.

What the page's tree for matching selectors costs, found later the same day (t3/page-tune):

- **`:has()` checked again after a key made the whole tree** (4502db0c). After each layout, what's drawn with what `:has()`
  said is checked again against the page (`checkHasWatched` from `laidOut`), and on shadcn's page and Mantine's that made
  `pageTree` again on every key, shadcn's 1,231 elements and 1,453 lists, half a megabyte: the tree was kept for it only if
  nothing an element's given had changed (`pageTreeContentKept`), and a field's value changes as it's typed into. Matching
  reads only where elements are and which have text from the tree (what each is given, its classes and state are read from
  what web.js keeps for it, in `ownOf`), so `pageTreeContentKept(true)` says whether that much is the same (`same_page_shape`
  in `geometry.rs`), which `recheckHas` and `drawnElementsOf` ask. Same exe, seven rounds, 40 keys: shadcn 183.6 -> 145.1
  ms with 3 small collections rather than 6, Mantine 301.4 -> 220.2 ms, 3 rather than 7. `RSUI_RESTYLE_CHECK=1` compares
  each answer with one from a tree made again (the `oracleTree` in `recheckHas`).
- **The tree made again from what's different in it** (53700f3c). With that, what asked for the tree next made it instead:
  Mantine's `document.querySelector('h1')` after a class went on it made all 4,500 elements, 4 ms, where it had been 0.06
  as the frame before had made it. Where the tree has the shape it had, `pageTreePatch` says only each element that says
  anything different of itself or is somewhere else, against where each was as it was last said (`RECTS`), and always the
  root, which carries the content's version; web.js puts each into the element made from that entry (`entryNodes`) and
  the text it gave (`textAt`), and works out what a selector reads of every element again. What isn't from Rust's tree
  (what isn't shown, a `<select>`'s parts, what a long list's rows skip) is said as a string as the tree is made, and if
  that or what the page has for any element's key is different the tree is made again (`pageNotTreeSame`). A tree that's
  kept is given back without asking `pageTree`. A class put on Mantine's heading and taken off: 3.68 -> 1.56 ms (the
  root and the heading said again), the class with the frame after it 6.5 -> 4.7 ms against t3/css-rust-finish, as the
  frame no longer makes the tree; shadcn's 40 keys 134 -> 126 ms (7 patches and one tree made, its MutationObserver
  reading `textContent`). `RSUI_PAGE_TREE_CHECK=1` makes the tree again after each patch and compares all of it;
  `RSUI_PAGE_TREE_PATCH=0` makes it again each time.
- **A focus made slower by putting what's about an element alone on together** (43f8c498). `orderWith` worked out, for
  each plan and list of rules found by an attribute, the order and whether each half could go on together, which for a
  plan used once, as most a focus makes are, was more than putting each on in turn. The first element with them now has
  them put on in turn with the order found as it goes (`cascadeOn`), only the last list asked about is remembered
  (`togetherAsked`, a string: a set for each plan was megabytes on Mantine's page), and the order is worked out the second
  time. Plans have what's worked out for them later there from the start (`lastSelf`, `selfLasts`, `together`,
  `togetherAsked`, `rustMatch`), as one shape. shadcn's focus on a field 15.1 -> 13.9 ms, its 40 keys 143.6 -> 139.9;
  with `profMark`, that part of a focus 16.7 -> 14.8 µs an element and of the page starting 15.2 -> 9.6, below the 16.1 it
  was before 6dd163a6. `RSUI_CASCADE_CHECK=1` works the order out at once, and puts the first element's on in turn too
  to compare (a copy that put no rules found by an attribute on that way had 27 differences on shadcn's page).

Mantine's "all steps" being 1,030 against 1,060 ms was settled with three builds of each (`__CARGO_DEFAULT_LIB_METADATA`),
five rounds in a turning order: 41416697 1,064 ms, this 904, each of its three builds below each of the other's; the one
step that was slower, the class put on its heading, is the one above, whose work had been in the `wait` after it, which
"all steps" doesn't count. Mantine's memory at its start moved by 8 MB between two web.js files that do the same (one
committed a step of pages more before a collection, with the same objects alive), and the other way in another build, so
memory is compared over builds and lengths of the environment.

The session, against the base before plans and matching were in Rust (9181f4e3), the commit before variables were worked
out in Rust (41416697) and t3/css-rust-finish's head (7227facb), fastrel exes, three builds of this, three of 41416697
and two of 7227facb, four rounds in a turning order on a busy machine, medians of steps (ms):

| | 9181f4e3 | 41416697 | 7227facb | This |
|---|---|---|---|---|
| shadcn's page: dialog opened, closed, opened again | 250.1, 164.4, 211.7 | 245.6, 157.4, 174.9 | 224.4, 131.2, 180.3 | 225.3, 126.9, 148.6 |
| shadcn's page: focus on a field, 40 keys | 18.1, 212.2 | 18.3, 234.8 | 23.1, 248.0 | 14.6, 132.2 |
| shadcn's page, all steps | 1,066 | 1,005 | 969 | 796 |
| Mantine's page: modal opened, closed, opened again | 134.8, 107.5, 123.5 | 125.7, 85.1, 97.1 | 95.1, 68.6, 84.3 | 81.4, 68.2, 81.5 |
| Mantine's page: a class on its heading; 40 keys; a turn of the wheel | 0.26, 475, 23.3 | 0.27, 471, 20.4 | 0.27, 432, 20.6 | 1.49, 273, 20.3 |
| Mantine's page, all steps | 1,434 | 1,326 | 1,175 | 1,028 |
| The explorer on System32: Down, Page Down, a resize; all steps | 1.74, 3.22, 3.82; 438 | 1.78, 3.83, 4.02; 457 | 1.78, 3.76, 4.02; 450 | 1.67, 3.28, 3.71; 421 |
| The long list, 5,000 rows, Down (eight rounds) | | 6.98 | 6.61 | 6.48 |

Private memory at the end, the median over eight lengths of the environment (MB): shadcn's page 133.0, 136.2, 137.9 and
131.6 (a second build of this 131.6); Mantine's 154.7, 153.0, 148.3 and 150.1, where 7227facb's builds are 146-148 at most
lengths and 154-156 at the others and this one's 147-151 at all (its second and third builds 150.2 and 149.9, a second
of 7227facb 148.1); the explorer 39.5, 40.4, 39.0 and 39.7, each within what lengths move it by (38.7-43.7); the long list
220.3, 220.5, 228.5 and 228.7. The long list's 8 MB more since 41416697 is committed and never touched: its private working
set is the same (226.0 against 225.7 MB), the process heap and the allocator's blocks are, and the engine has as many
objects alive (1,234,713 places of objects rather than 1,226,014). That looks like one of the engine's lists growing to
twice its room as it goes past a length, which 5fb3a79b on t3/gc-tune (asking for at most a megabyte ahead) is about; it
wasn't looked into further here, as that's the collector's.

What made the full collections, found later the same day: most of what full collections freed was made old by mistake.
A closure's variables and strings were old the first time a small collection kept them, where an object waits for the
third. `pageTree` in web.js makes the page's tree again on every key (1,231 nodes and 1,453 entries, about half a
megabyte), and a small collection that came while it was building kept its variables, which were then old; once it had
returned, they still held that tree, and an old variable that holds something young is looked at by every small
collection until that's old too, so the tree became old and died old, at nearly every small collection while typing
(`RSUI_GC_DIED_OLD=1` shows it, by what each small collection makes old and each full one finds dead). Variables and
strings now stay young until two small collections have kept them (see `cells_surviving` in rsui-js's `gc.rs`). Over
eight lengths of the environment, against the base before plans and matching were in Rust (9181f4e3) and the last
commit before this (41416697): shadcn has no full collection in its 40 keys on any length (base 2, before 8), 14 full
collections in the session rather than 15 and 16, 89 ms of them rather than 121, and small collections make 47.5 MB old
rather than 64.6 and 57.6. Private memory at the end, medians: shadcn 131.8 MB (132.6, 134.7), Mantine 148.7 (154.1,
152.8), the explorer 41.0 (39.4, 40.7) and `bench/long-list` 223.3 (220.5, 220.2), which is where its last small
collection lands: 20 keys later it's 223.4 against 231.3 and 231.4. Lists of the engine also ask for at most a megabyte
ahead of what they need, where an eighth of the long list's objects was 10 MB of private memory nothing had touched.

Why the explorer ends 1.3 MB higher than the base, and the `json` bench peaks 1.6 MB higher, found after: it isn't the
allocator (its chunks hold 2.0 MB more than their blocks in use in both builds), but when full collections land. With
less made old, the explorer's last full collection is the one during startup, and what it keeps there is its working set
too, all made old; the 1.5 MB that browsing makes old afterwards stays short of what makes the next one due. A full
collection that leaves what's young young fixes both (39.7 MB, json 10.1) but makes collecting 12-27% slower in the apps
and the long list's keys a third slower, so it's an experiment, off by default: rsui-js's `DESIGN.md` has what was
tried and the numbers.

Both together (t3/all-tuned, with `Vm` laid out as before again, see rsui-js's `DESIGN.md`), against 8c9786d3, fastrel,
two builds a side, eight runs a side on a quiet machine: all of shadcn's steps 567 ms against 782, Mantine's 737
against 1,111, the explorer's 366 against 413, the long list's 293 against 312; started and closed, Mantine 2,072 ms
against 2,638 and shadcn 1,650 against 1,949; Mantine's class put on and taken off, with the frame after, 2.8 ms
against 4.8. Two steps are slower. shadcn's focus on a field (14.3 against 11.3 ms): a small collection of 4-5 ms
lands in it on every length of the environment tried, where without one it's 10 ms, and the session has 51 small
collections rather than 71, 334 ms of collecting rather than 513. An `expect` step, 0.5-1 ms on both pages, which
3d504951 found is the exe and not what it runs. Private memory at the
end, medians over eight lengths: shadcn 131.2 MB (133.0), Mantine 146.0 (154.0), the long list 221.7 (220.4) and 20
keys later 222.7 (231.2), so the 8 MB the plans had cost it are gone; the explorer 40.9 (40.2). After an `idle` step,
as a window does when things go quiet, its JavaScript keeps the same 6.2 MB as page-tune's, and the 1.7 MB more than
page-tune is the engine's list of objects (73,600 places rather than 69,400, 54 empty pages given back rather than
116) and the allocator's chunks around it: with less made old, its dead old objects pile up higher before a full
collection, and a list that's grown doesn't shrink past the pages still partly used.

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

### Where the memory goes

Measured on 2026-11-20 with the `regions` step (`process_memory.rs`), which puts each reservation of the process's
addresses down to what it's for, and `memory`, which says what the engine's lists, the allocator, the span recorder, the
inspector's frames and tracing keep. Headless, with the steps of the bench scripts (shadcn's and Mantine's pages, the
explorer on System32, the long list of 5,000 rows), at their end, in MB, before this round's changes (3253d551):

| | shadcn | Mantine | explorer | long list |
|---|---|---|---|---|
| Private bytes | 132.4 | 145.9 | 38.1 | 220.6 |
| rsui-alloc's chunks (blocks of 1 KB or less), of them in use | 52.2, 37.5 | 64.8, 55.8 | 11.7, 9.7 | 98.6, 89.4 |
| The process heap committed, of it allocated | 36.7, 28.2 | 34.8, 31.5 | 15.8, 11.4 | 31.7, 27.5 |
| The engine's list of objects, of it free places | 24.0, 10.0 | 30.0, 3.3 | 3.9, 0.5 | 78.0, 4.7 |
| The engine's list of strings | 5.6 | 8.0 | 1.5 | 7.1 |
| The engine's code: instructions, caches, prototypes, constants | 11.0 | 4.4 | 2.7 | 2.6 |
| Parse trees, threads' stacks, the program's pages made private | 1.6 | 1.5 | 1.5 | 1.1 |
| JIT code (a section of the page file, so not private bytes) | 7.6 | 8.4 | 4.1 | 3.1 |
| Working set, of it private | 157.7, 126.5 | 169.6, 138.2 | 57.2, 34.5 | 235.9, 216.7 |

The working set has the program's own pages (7.5 MB here, a build without LTO), Windows' DLLs (7-8 MB) and the JIT's code
twice, once for each of its two views. The rest of where it goes:

- **Tracing renders.** Under `--do` and `--agent` the inspector keeps what renders and why from the start, which was 11 MB
  of shadcn's page, 18 of Mantine's, 7 of the explorer's and 10 of the long list's (`RSUI_INSPECT=0` turns it off): the
  span recorder at its `Detail` level, the inspector's frames (a render is 120 bytes, with its path and why), a string
  for each hook of each component saying where it's called from, and the sentences saying why each render happened.
- **JavaScript objects for each element.** The long list's 25,000 elements are 4.8 KB of the engine's objects each: a
  `PlacedLive` (544 bytes, with 60 properties), seven `PassedHere`s for the contexts a `Styled` passes down, two component
  instances, three of React's elements, a look, a `Map` of what it read, and a dozen smaller ones. Rust keeps another 1.4
  KB for each (its props as a `ThinArc`, the runtime's slots, layout's answers). That's most of the long list's memory,
  and the floor below what's here needs fewer and smaller objects for each element in `web.js` and `layer.js`.
- **Room between collections.** A small collection comes after three-eighths as much again as what's alive has been made,
  which is the 40% of free places in the list of objects after one: the long list makes 34 MB between small collections.
- **What loading leaves.** Finding modules kept every folder's listing (3 MB on shadcn's page, two 1 MB ones for the
  explorer's icon package), the compiler kept a map of every constant (1.1 MB), and the shapes' `Vec` doubled (9 MB
  committed for Mantine's 3.3 MB of shapes).
- **The same text many times.** Strings with text of their own were 1.8 MB on the long list, 1.6 MB of it the same as
  another's (its rows' `className`s), and 2.2 MB of Mantine's 3.4.

What was done about it (t3/memory-floor): tracing keeps each hook's call site and each reason a render gives once, and
the span recorder keeps its spans packed in pieces (`rsui-spans`); what loading leaves is let go of between frames
(`Engine::let_go_of_loading_caches`); the shapes' list doubles only up to a megabyte; strings with the same text share it
once a collection keeps them (see rsui-js's `DESIGN.md`). Private memory at the end, the median over eight lengths of
the environment, against 3253d551, in MB:

| | shadcn | Mantine | explorer | long list |
|---|---|---|---|---|
| Traced, as under `--do` | 121.4 (133.3) | 129.8 (144.2) | 35.2 (40.5) | 217.4 (220.6) |
| `RSUI_INSPECT=0`, as in a window | 114.8 (121.9) | 118.6 (123.4) | 31.9 (35.2) | 209.9 (211.5) |
| Peak, traced | 123.5 (133.3) | 135.0 (146.1) | 35.2 (40.5) | 217.4 (220.6) |
| A second after starting, traced | 118.3 (119.0) | 124.8 (137.6) | 27.6 (30.0) | |
| Working set at the end, traced | 147.6 (157.7) | 156.5 (167.7) | 54.8 (59.4) | 232.4 (235.7) |

Not done, and what it would take: `RSUI_GC_YOUNG_CAP=16` (megabytes made between small collections at most) has the long
list end at 195 MB and `=8` at 186, and Mantine at 120 with `=8`, but it's 4-6% more CPU on shadcn's page and the long
list, so it's left off. Below that, the floor is the elements' JavaScript: a `Styled` element as one object with its
state in a few typed fields rather than a `PlacedLive`, a `PassedHere` for each context and two instances would be most
of the long list's 120 MB of objects, and an object of 48 bytes rather than 64 (the JIT's code takes a handle shifted by
6 to be an address) another 20 MB of it.

The second round (t3/memory-to-the-floor) took what each of a page's elements keeps in JavaScript down, as the census
(`census` in steps, with `--features alloc-owners`) found it on the long list's 25,000 elements:

- **What's passed down is only passed where something could read it.** A `Styled` element whose children are only text
  passes nothing down: no `PassedHere` for each of its seven contexts, no rules' chain worked out for what's in it. It
  passes from the first render where something in it could read them (a component, a page's tag written as a string, a
  host element given a function that isn't a handler, what a portal or code put in it, or anything a `styled(Component)`
  renders), and on every render from then on, as `provideHere` needs (`holdsComponent` in `web.js`).
- **One provider tells a line it's a line**, around the line's group, rather than a `FlowContext` provider, with its
  instance, element and props, around each piece of it (`wholeLine`). Where a line goes through `Children.toArray` (a list
  item's marker, what's beside a float) or is held before what's on it is known, it keeps one around each piece, and what
  finds an element among what's beside it for code moving it looks past the line's provider (`justInside`).
- **`PlacedLive` is made with ten properties**, not twenty-one that grew to fifty-eight: what few elements have (its ref,
  whether it's mounted, what it passes down worked out, its handlers' functions) is added once there's something, its
  `handlers` and `instance` are getters, and what `Styled` set each render to an empty default isn't written until it's
  something.
- **What an instance read of a context** is a list of each context and what was read, inside its own object, not a `Map`.
- **A string prop's value** is shared with the last element given the same text (`shared_text` in `props.rs`), so each
  row's class names are one `Arc<Text>` between them.

Private memory at the end, the median over eight lengths of the environment, against 4ea83414, in MB:

| | shadcn | Mantine | explorer | long list |
|---|---|---|---|---|
| Traced, as under `--do` | 120.7 (121.3) | 127.7 (129.7) | 35.4 (35.5) | 162.9 (217.6) |
| `RSUI_INSPECT=0`, as in a window | 113.9 (115.1) | 115.4 (118.7) | 31.9 (32.1) | 159.4 (209.8) |
| Peak, `RSUI_INSPECT=0` | 118.2 (118.9) | 120.1 (122.7) | 32.6 (32.6) | 159.4 (209.8) |
| A second after starting, `RSUI_INSPECT=0` | 111.6 (112.1) | 114.3 (116.2) | 27.2 (27.0) | |
| Working set at the end, `RSUI_INSPECT=0` | 139.9 (140.6) | 143.4 (146.1) | 52.1 (52.0) | 174.4 (223.3) |

The long list's engine objects are 3.7 KB an element where they were 4.8 (91.6 MB where they were 120.7, with the room
between collections), and rsui-alloc's blocks in use 63.0 MB where they were 86.8. The page apps' memory is mostly
elsewhere: on shadcn's page 17 MB of rsui-alloc's 54 MB of chunks is room between blocks, spread over every size class
(`allocator classes` in `memory`; taking blocks from the fullest chunks first changed nothing), 9 MB of the list of objects
is room between small collections, and the process heap keeps 8 MB more than it's using (`HeapOptimizeResources` gave
none of it back).

None of it is slower. Process CPU over a session with `RSUI_INSPECT=0`, medians of ten alternating runs: shadcn's page
2898 ms (2883), Mantine's 3383 (3477); of six: the explorer 805 (844), the long list 2656 (2750). To start, of eight: the
long list 1719 ms (1922), the others within a percent. js-bench's time and peaks are as they were (four salted builds a
side: 0.989 of base's time over the interpreter's benches; peaks' geomean 5.148 MB against 5.139).

Tried, and not kept: `RSUI_GC_YOUNG_CAP=8` takes the long list to 142 MB, Mantine to 109 and shadcn to 112, but with the
collector's other costs cut (see rsui-js's `DESIGN.md`) it's still 5-7% more CPU on the explorer and the long list; a cap
only while small collections find mostly garbage hardly ever applied by the end of a session. Compiling `node_modules`'
functions when they're first called (`LazyCompile::NodeModules`) had shadcn's page 20 MB higher, as what's waiting keeps
its parse tree. An object of 48 bytes needs three properties inside it rather than four and a `Kind` of 16 bytes with its
tag in the object's header, which is every `match` on an object's kind, and the JIT's addressing (`handle << 6`) besides.

The third round (t3/push-memory-floor-lower) looked at what isn't the elements' JavaScript, and kept three things:

- **The code lists' gaps give their pages back.** A full collection frees the code of what can't run any more, which is
  mostly files' top levels, as each runs once; that left 286K of shadcn's 655K instructions and 61K of its 145K property
  caches as gaps that kept their memory. The whole pages inside a gap are decommitted now (see rsui-js's `DESIGN.md`).
- **A page element's `dom_key` goes beside a set of props that's shared.** No two of a page's elements had the same props,
  as each has its own `dom_key`, so the sets bridge.rs keeps for elements alike to share (`RecentProps`) never matched, and
  each of the long list's 20,000 rows and cells had a set of about 300 bytes. Now `native_props` looks a set up without
  its `dom_key`, and the props are a 64-byte block with the key and the shared set (`Props::with_dom_key` in rsui-core):
  still one pointer, so an element's still 56 bytes, and everything reading props still finds `dom_key` among them.
- **At most 12 MB is made between small collections** (`RSUI_GC_YOUNG_CAP`, see rsui-js's `DESIGN.md`), which only binds
  above 32 MB alive.

Private memory, the median over eight lengths of the environment, against 9ca439f0, in MB (`si`/`mi`/`fxi`/`lli` steps for
`RSUI_INSPECT=0`, the bench scripts' for traced):

| | shadcn | Mantine | explorer | long list |
|---|---|---|---|---|
| End, `RSUI_INSPECT=0` | 110.0 (112.7) | 110.4 (115.0) | 29.7 (29.6) | 132.5 (147.7) |
| Peak, `RSUI_INSPECT=0` | 117.2 (118.9) | 113.6 (117.9) | 32.4 (32.4) | 143.0 (164.4) |
| A second after starting, `RSUI_INSPECT=0` | 110.6 (111.7) | 109.3 (114.5) | 27.0 (27.0) | |
| Working set at the end, `RSUI_INSPECT=0` | 136.3 (138.9) | 138.1 (143.1) | 50.0 (49.9) | 148.3 (162.7) |
| End, traced | 119.3 (120.8) | 125.5 (128.4) | 33.6 (33.4) | 141.9 (162.8) |
| Peak, traced | 123.6 (125.3) | 127.4 (132.4) | 33.6 (33.4) | 141.9 (162.8) |

Of the long list's 15 MB, the shared props are 5 and the cap 10; of shadcn's 2.7, the code's gaps 2.2; Mantine's is the
cap. Process CPU over a session, `RSUI_INSPECT=0`, two salted builds a side, medians of twelve alternating runs: shadcn's
page and the explorer as base, Mantine 0.3% more, the long list 1.8% more, of which 1.3% is the cap (one more full
collection as it builds its rows, 19 ms, and more small ones; with `RSUI_GC_YOUNG_CAP=16` the long list ends 4.7 MB lower
rather than 10 and costs nothing, and Mantine's as before). Starting, the long list is 2% slower for the same full
collection; shadcn's page is 1.5% slower, but by as much in a build of base with only measuring code added, so that's how
the program's laid out. js-bench's peaks are as they were (geomean 5.13 MB both), and its time is too, against the faster
of base's two salted fat-LTO builds (the other is 15-25% slower on every bench, which is the salt).

Measured and left:

- **Compiling a function when it's first called, from its text.** `RSUI_LAZY_COMPILE=all` (a knob for measuring) shows
  what never runs: shadcn's code lists are 8.0 MB rather than 11.0 (instructions 3.5 rather than 5.2), Mantine's 3.2
  rather than 4.3, the explorer's 1.3 rather than 2.7, the long list's 1.6 rather than 2.5. Getting that without the parse
  trees means parsing one function again from where its text is, with what the parser knew there: whether the file's JSX
  is a page's (decided for the whole file, by reading it twice), the private names of the classes around it, strictness,
  and the numbers it gives functions. That's a parser of its own to keep the same as the other, for 1-3 MB.
- **Whole pages of rsui-alloc's chunks that nothing uses** (`RSUI_FREE_PAGES=1`): 2.8 MB of shadcn's 16 MB of room, 2.3 of
  Mantine's, so the room is free blocks between blocks in use on almost every page, and giving pages back isn't where it is.
- **`RSUI_EXACT=1`**, shrinking objects' storage to fit after full collections: Mantine 2.6 MB and the explorer 1.7 lower,
  but the long list 6 MB higher, as the blocks move to smaller classes' chunks and leave the old ones with room.
- **Waiting longer for full collections that find nothing dead.** The long list's start has eighteen, each of them finding
  1-3% of what had become old dead. Making the next wait up to four times as long cut its collecting by 15 ms, but
  Mantine's page then ended 7.5 MB higher, as its full collections while it starts do find what's let go of.
- **A cap of 8 MB**: 3 MB more off the long list and Mantine's page, 2% more CPU on shadcn's page and the long list.

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
