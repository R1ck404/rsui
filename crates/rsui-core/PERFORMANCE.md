# Performance notes

Results of the speed and memory experiments, so they don't get repeated. Everything was
measured on one Windows 11 machine. Speed comes from
`cargo bench -p rsui-core`, on a tree of about 10,000 nodes or a flat keyed list of 10,000
items, with the default system allocator. Run-to-run noise on heap-allocated trees can reach
about 20%, so small differences mean nothing.

## Memory

### Window

The renderer is custom Direct3D 11, so this is the largest cost. All of these are 1920x1080 windows, in MB, on an
NVIDIA RTX 5070. Other GPUs will differ.

| Setup | Private, after startup | Private, trimmed and rendering | Working set, trimmed | GPU, attributed |
|---|---|---|---|---|
| Direct3D 11 defaults | 9.5 | 1.6 | 4.0 | 28.4 |
| Lean flags, flip swap chain | 9.4 | 1.4–2.1 | 3.6–6.3 | 28.4 |
| Same, with 5,000 quads and a 1 MB atlas | 10.9 | **2.0–2.4** | **4.4–7.0** | **29.5** |
| Composition surface, with the same drawing | 10.8 | 2.0–2.3 | 4.6–6.8 | 29.6 |
| Direct3D 12 | 40.7 | 2.4 | 5.2 | 37.1 |

**The setup to build on:**

- Direct3D 11 with `D3D11_CREATE_DEVICE_SINGLETHREADED` and
  `D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS`.
- A flip swap chain with two buffers.
- `IDXGIDevice3::Trim` and `K32EmptyWorkingSet` after a second or so of inactivity. Most of the
  startup memory is never touched again.
- `ResizeBuffers` down to 1x1 while hidden, which took attributed GPU memory from 28.4 to 11.6 MB.
- Compiled shaders shipped with the program, so the shader compiler is never loaded.

**What the numbers mean:**

- Starting the GPU used to peak at 42-47 MB of working set, for under half a second. A limit on the
  working set now holds it to about 18 MB. See below.
- About 50 MB stays committed, which counts against the system's limit but isn't using RAM. Nearly
  all of it is NVIDIA's driver: 31.5 MB of writable DLL sections, of which `nvwgf2umx.dll` is 24.6 MB,
  and most of the rest is private memory it allocates. The limit doesn't change this.
- Trimming blocks for about 16 ms, about one frame, so it should only happen when idle. The next
  frame adds about 900 page faults and, where it could be measured, up to about 3 ms.
- At 1080p the true GPU cost is about 25.6 MB: 16.9 MB for the buffers and about 8.8 MB for the
  driver. The buffers scale with the window, so 4K needs about 66 MB for them.
- The figures don't include text beyond the 1 MB atlas, and rendering touches more pages as more
  code runs, so a real app's trimmed figures will be higher.

**What didn't help:**

| Idea | Result |
|---|---|
| Composition surfaces | Attributed 16.9 MB less to `dwm.exe`, but that was the same buffers counted twice. In the cleanest runs the adapter total rose by 25.6 MB either way |
| Composition swap chain | No change to attributed GPU memory |
| Feature levels 10_0 and 9_3 | No change |
| Frame latency of 1 | No change |
| One buffer with blit presentation | No change to video memory, and more committed memory |
| The AMD integrated GPU in this machine | Worse: 26.4 MB private and 39.7 MB of video memory, against 8.9 and 24.2 |

The adapter and `dwm.exe` totals include every other app, so they swing by hundreds of MB. Only
trust repeated, matching readings.

**The limit while the GPU starts:**

`WorkingSetLimit` sets a hard maximum on the working set while the driver loads, and lifts it after
the first frame. Older pages leave RAM as new ones are touched. Results for the TSX gallery, in a
small window, taking the middle of 5 runs:

| Room above what was in RAM | Peak working set, MB | First frame, ms |
|---|---|---|
| No limit | 43.6-47.4 | 215-257 |
| 8 MB | 22.1 | 255 |
| 6 MB | 20.1 | 257 |
| **4 MB, the setting used** | **18.2** | **261-296** |
| 3 MB | 17.3 | 288 |
| 2 MB | 16.2 | 314 |
| 1 MB | 15.3 | 303 |

Runs are noisy: with no limit, single runs ranged from 180 to 393 ms. Over 8 runs each of the final
build, the middle time from starting the GPU to the first frame was about 30 ms longer with the limit.

The limit also has a minimum of 18 MB in total. Coming back from deep idle starts with almost nothing
in RAM, so room alone would squeeze it. With the minimum, coming back peaked at 18.0 MB and not
29.6-32.5 MB, and took 152-153 ms where no limit took 139-181 ms.

In the final build, the peak is 18.2 MB for the TSX gallery and 18.1 MB for the Rust one, with 53.8 MB of
peak private bytes for each. The limit had ended in all 5 runs of each, and read from outside the process
after coming back from deep idle, it had ended then too. Once it's gone, a window that's been used and
come back from deep idle has reached 19.0-19.4 MB at most.

What else was measured, for the TSX gallery with the limit on:

- About 14 MB is in RAM before the GPU starts, which is why the peak can't easily go lower. Only about
  2 MB of it isn't shareable. The rest is mostly Windows DLLs that other programs share.
- Idle is 0.2 MB of working set. Woken from idle by mouse moves, it's 4.6 MB, with 1.8 MB not
  shareable. The Rust gallery is 4.7 MB and 1.8 MB, so the JavaScript engine adds almost nothing here.
- Before the first idle trim, other things briefly bring it to 7-10 MB, mostly text input DLLs and
  UI Automation.

What didn't help:

| Idea | Result |
|---|---|
| Emptying the working set before starting the GPU | Peak of 14 MB, but the first frame took 333-486 ms and much more CPU, as the driver needs many of those pages |

Only one limit should exist at a time, as ending one ends them all. `ShowWindow` can start the GPU
itself, which caught out an experiment that ended a limit just after calling it.

### Tree

Real bytes per node, from the growth of the process's physical memory:

| Node | Start | Now, Windows heap | Now, `rsui-alloc` |
|---|---|---|---|
| Interactive control, shared props | 378 | 91 | **74** |
| Label up to 15 bytes, or a group | 170 | 91 | **73** |
| Interactive control, own props | 378 | 169 | 137 |
| Longer label | 250 | 187 | 153 |

A 10,000-node tree of controls with shared props takes about 0.74 MB, down from 3.8 MB.

What brought it down:

- **`rsui-alloc`:** a custom allocator with no per-block overhead. It adds about 44 KB when idle,
  and building a tree took about half the time (2,820 µs to 1,371 µs). Mimalloc gave the same
  node sizes but added 1.3 MB when idle, and 169 bytes for a longer label.
- **Compact fields:** an `Element` went from 136 to 56 bytes. It uses `triomphe::Arc` (no weak
  count), `EcoString` (15 bytes inline, and empty means no name), `ThinVec` for children,
  interned custom roles and action sets, and thin string keys.
- **Shared prop sets:** `Props` is an immutable array in one allocation, and `shared_props!`
  builds one per call site. This nearly halves a control's memory.
- **Interned prop names:** each entry went from 48 to 24 bytes.

Things to know:

- `rsui-alloc` only reuses a freed small block for its own size class, and never returns chunks
  to the system. Trimming still takes unused pages out of RAM.
- `Symbol`, custom roles and custom action sets are never freed, so they mustn't be built from
  unbounded input.
- `Props::insert` copies the set, which made `.prop()` chains about 35% slower to build than
  with a growable array. Prefer `shared_props!` or `Props::from([...])`.
- Rebuilding a tree holds two trees in memory until the old one is recycled, and `NodePool`
  keeps free nodes until `shrink_to`.

## Runtime

Measure with `cargo bench -p rsui-runtime`, which runs on `rsui-alloc`. Each row
below has one piece of state and one click handler.

| Operation | Time |
|---|---|
| One row changes, in a list of 10,000 | **0.32 µs** |
| Scroll a virtual list of 1,000,000 by one row | 11 µs |
| Mount a virtual list of 1,000,000 | 9.5 µs |
| Parent renders again, 10,000 rows unchanged | 493 µs |
| Reverse 10,000 rows, keeping their state | 963 µs |
| Mount 10,000 rows | 2,203 µs |

| Requested memory | Bytes |
|---|---|
| A row as an element only | 72 per row |
| A row as a component | 204 per row |
| A row with state and a handler | **316 per row** |
| A virtual list of 100,000 rows, with 50 built | 38,469 in total |

Requested bytes leave out allocator rounding, and some of the virtual list's total is
pre-reserved instance storage that hasn't been touched.

What made the difference:

- **Updating from the changed component:** a one-row update diffs that row and updates the tree in
  place along its path. Instances don't hold their output, so those nodes have one owner.
- **Finding a handler with a binary search:** this took a one-row update from 16 µs to 0.32 µs,
  because `dispatch` had been scanning all 10,000 siblings.
- **Exact capacity for hooks and handlers:** this took a row from 543 to 316 bytes, together with
  storing instances in chunks, which also avoids two copies existing while the storage grows.
- **Confirming children are in order:** this avoids sorting every child's address when a parent
  renders again. Release builds fall back to sorting if they aren't in order.

`NodePool` isn't used by the runtime. Elements are built with the ordinary builder, and
`rsui-alloc` already makes allocating them cheap. If whole-tree rebuilds turn out to matter, the
pool's remaining gain is about 28% (454 µs against 628 µs), which could be revisited then.

## Layout

Measure with `cargo bench -p rsui-layout`, on `rsui-alloc`. Each of the 10,000 rows is a component
holding a text element, in a list.

| Operation | Time |
|---|---|
| One row changes, from the action through layout | **0.52 µs** |
| Nothing changed | 0.013 µs |
| First layout | 4.1 ms |
| Window resized | 5.6 ms |

Resizing lays everything out again, so it costs more than the first layout's 4.1 ms, which suggests
it's worth looking at if resizing feels slow. These rows aren't in a virtual list. Each node also has
a 56-byte layout box, and a parent has a 16-byte header for its boxes, so a plain element with its box
is about 130 bytes.

What made the difference:

- **Patches instead of diffing:** layout finds out what changed from the runtime's patches, so an
  update never walks the tree looking for changes.
- **Stopping early:** a change that doesn't alter a node's size stops at that node. `Layout::last_solved`
  reports how many nodes an update really touched, and tests check it, because a layout doing too
  much work still comes out right. This check found that stopping early had never worked at first.
- **A hint for the dirty child:** finding the one changed row among 10,000 took most of the time, at
  about 30 µs. A single remembered index took the whole update to about 0.5 µs, and fits in padding.
- **Remembering displaced answers within a pass:** across 20,000 random deep trees, the worst case
  was 170 nodes solved with this, and 573 without.
- **Patches for many rows at once:** rows added or removed one at a time moved everything after each of
  them. Filtering 10,000 rows down to half and back went from 20.1 ms to 4.5 ms.
- **Handing styles down:** a parent reads each child's style anyway, so the child no longer reads it
  again. Laying out 10,000 rows for the first time went from 4.20 ms to 3.08 ms, and resizing from
  5.33 ms to 4.22 ms, with shipped build settings and best of 5 rounds, on a machine that was busy.

Stack limits how deep a tree can be, as a real program's main thread has 1 MB. Layout uses 818 bytes
per level, down from 1,363, and painting about 450, so a tree of 1,200 levels works and 1,300 doesn't.
Keeping a style out of the frame that goes down a level is most of that, so anything big added to
`solve_anew` or `solve_flex` costs depth. Debug builds use far more stack, and only test 200 levels,
so run `edge.rs` with `--release` after changing them.

The guarantee everything relies on is that updating a layout gives exactly what computing it from
nothing would. Random trees and changes check this, at 3,000 cases per test, and it held at 40,000.

## Text

Measure with `cargo bench -p rsui-text`.

| Operation | Result |
|---|---|
| Measure a label | 0.13 µs |
| Draw a 14 px glyph | 1.7 µs |
| Lay out 10,000 labels | 2.5 ms |
| A font in physical memory, mapped | 104 KB, against 1,016 KB read in |
| Glyphs that fit a 256x256 atlas | 376 |

The atlas keeps no copy of its pixels on the CPU. Results were compared against WPF as an independent
reference: glyphs and advances agree across 8 fonts, and kerning is exact on all 2,459 pairs checked
for Segoe UI, Arial and Consolas. Other fonts differ on some pairs, such as hyphen pairs, ligatures,
and Tahoma's older `kern` table.

### Shaping

Right-to-left text, joined letters, ligatures and marks cost plain text almost nothing, because plain
text never goes near them. All it pays is one check for each piece of text. Measure with
`cargo bench -p rsui --bench frame -- shaping`.

| Operation | Before shaping existed | Now |
|---|---|---|
| Paint the dashboard | 48.5 us | 48.8 us |
| Paint 2,000 labels | 92 us | 98 us |
| Paint a 100 KB paragraph | 207 us | 207 us |
| Resize 2,000 labels | 1,578 us | 1,590 us |
| A key in a text input holding 1 MB | 9.6 ms | 9.3 ms |
| The gallery's program file | 532 KB | 717 KB |

And text that does need it, against 0.16 us for plain text of the same length:

| Operation | Result |
|---|---|
| An Arabic sentence, the first time | 12 us |
| A line of Hebrew, the first time | 27 us |
| Either, once it's been seen | 0.03 us |
| A 100 KB Arabic page, the first time | 18 ms |
| The same page, each frame after that | 7 us |

Results are remembered, in at most 512 KB, unless one text alone needs more, and all of it goes when
things go quiet. What made this fast:

- Each lookup keeps a summary of the glyphs it could apply to, in 128 bits. Most lookups have nothing
  to do with most glyphs, and this made shaping 5 to 6 times faster.
- Long text remembers the lines it last shaped, which took a page from 8 ms each frame to 7 us.
- The check on plain text looks at each byte only if the font has ligatures for ASCII at all.

A font can't make this hang: everything read from it is checked, and running it has a budget for each
glyph, a limit on how far rules can call each other, and a limit on how many glyphs a run can grow to.

All of this was compared with DirectWrite, which shares no code with it, in 10 fonts
over 3,400 strings for each font that can show them. Of 21,719 comparisons, 21,586 are the same in
glyphs and positions. Those that aren't are sequences that don't occur in real text, such as a dagesh
on an ayin, or several kinds of marks on a letter around a joiner. These differences are on purpose,
each following HarfBuzz, and the tool allows for them:

- DirectWrite leaves marks in the order they come. rsui sorts them into their standard order first.
- DirectWrite can leave a mark off its anchor if the font kerns after placing marks. rsui keeps it on.
- DirectWrite's shaping ignores the old `kern` table, in fonts like Tahoma. rsui uses it.
- DirectWrite kerns against joiners as if they were letters. rsui looks past them.
- DirectWrite lets a joiner stop a rule that's only looking at what's around a letter. rsui passes over
  it unless it's what the rule is looking for, which is the difference in 28 strings of Arabic with
  joiners in them, all of which come out the same as HarfBuzz.

### Every other script

Indic scripts, Khmer and Burmese write some vowels and consonants in a different order from how they're
typed. Each script has its own rules, in `indic.rs` and `southeast.rs`. Thai and Lao need less, as their
letters stay in order. All of it is compared with HarfBuzz, glyph for glyph and position for position:

| What | Strings | The same as HarfBuzz |
|---|---|---|
| Devanagari, Bengali, Gurmukhi, Gujarati, Oriya | 14,912 | all |
| Tamil, Telugu, Kannada, Malayalam, Sinhala | 14,677 | all but 1 |
| Khmer | 2,044 | 99.95% |
| Burmese | 2,579 | 99.77% |
| Balinese, Javanese, Tagalog, Buginese, Chakma, Meetei, Brahmi | 11,740 | 99.7 to 100% |
| Thai and Lao | 22 | all |
| Emoji, including every one in Unicode's list | 1,262 | all |

Real sentences match in every one of them, and what's left is text that can't occur. The rarer scripts
share one set of rules, the Universal Shaping Engine, in `universal.rs`, with tables made from Unicode's
own data files. A few signs in them differ from those files, the same way they do in HarfBuzz: these were
found by asking it which pairs of signs it takes. Fonts for these scripts come with Windows for only some
of them, and an app can add others with `with_fallback`.

`tests/harfbuzz.rs` keeps a sample of these, with what HarfBuzz made of each, so they don't need
HarfBuzz to run. They only run with the same version of the font, and say so if they can't.

What this cost everything else, measured with the settings that ship, from worktrees, taking turns:

| Operation | Before | After |
|---|---|---|
| Paint 10,000 boxes, dashboard, 2,000 labels, a 100 KB paragraph | | the same |
| Arabic or Hebrew that's been seen | 0.026 us | 0.026 us |
| An Arabic sentence, the first time | 11.8 us | 12.3 us |
| A line of Hebrew, the first time | 26.2 us | 27.6 us |
| A 100 KB Arabic page, the first time | 17.9 ms | 18.4 ms |

The first time costs a little more because each glyph's record while shaping went from 28 bytes to 32,
to hold what these scripts need to know about it. A test keeps it from growing.

### Fonts for every language

`TextEngine::with_system_fallbacks`, which `default_text_engine` uses, knows which fonts that come with
Windows have what. Each is only opened when text first needs it, and fonts are mapped from their files,
so opening them is cheap, and ones that aren't needed cost nothing but their names:

| What | RAM |
|---|---|
| Knowing about all 14 fonts, with none open | under 2 KB |
| Opening one for Chinese, Korean, Thai or Burmese | about 75 KB |
| Opening the one for emoji | about 60 KB |
| Opening the one for Hindi, with its shaping rules | about 210 KB |
| Windows's dictionaries, the first time Thai, Lao, Khmer or Burmese has to wrap | about 2 MB |

Text that needs no new font pays one step for each character that isn't ASCII, and nothing if it's all
ASCII. Once they're all open, or if none were asked for, it's one check for each text.

Which font wins doesn't depend on which text came first: each font has a place in the order, which
stays the same however late it's opened. Han characters look different from one country to the next,
so the font for the user's own language goes first.

Lines in Thai, Lao, Khmer and Burmese can only break between words, and these have no spaces, so that
takes dictionaries. Those come from Windows's own copy of ICU, in `icu.dll`, loaded only when a paragraph
in one of these actually has to wrap. Without it, Thai and Lao still break before a vowel that goes first.

### Color emoji

An emoji is drawn with gradients, from version 1 of the font's `COLR` table, by a small software painter
in `color.rs`, once at each size it's wanted in. It's then a picture like any other, through the same
image path with no new shader, kept until it's gone unused. Text with no colored glyphs has one check for
each text once a font with them is open, and none before that. Fading text fades its emoji without drawing
it again. Fonts that only have version 0 have their flat layers go through the atlas like any other glyph.

Side by side, these come out the same as Edge draws the same font. Windows's emoji font has no flags of
countries, and shows their letters in their place, as it does everywhere on Windows, so that's what they
are here too. A font with them, like Noto Color Emoji, gives them with one line, leaving other emoji alone:
`with_lazy_fallback(path, &[0x1F1E6..=0x1F1FF])`, before `with_system_fallbacks`.

### Scrolling

Scrolling only moves what's inside, so nothing is measured again, and what's out of view isn't drawn.
Measure with `cargo bench -p rsui --bench frame -- scrolling`.

| A notch of the wheel, up to what's drawn | Time |
|---|---|
| 100 rows | 25 us |
| 1,000 rows | 93 us |
| 10,000 rows | 980 us |

Layout tells the painter which elements cut off what's inside them, which saves reading every element's
props to find out. Together with inlining, that took painting 10,000 boxes from 347 to 279 us.

## Images

`rsui-image` reads PNG and JPEG with no dependencies. An image only exists on the GPU, at the size it's
shown, and is read again from its file if it's needed at another size, or the GPU has been let go of.
Reading happens on a thread that's gone once there's nothing to read.

| Reading a 3840x2400 JPEG of 1.5 MB | Time | On the GPU |
|---|---|---|
| In full | 118 ms | 36.8 MB |
| At 384x240 | 19 ms | 369 KB |
| At 48x30 | 18 ms | 5.8 KB |

800 files from Windows came out the same as Windows' own decoders, 765 exactly, and 35 within a rounding
error, the worst having a mean difference of 0.04 out of 255. They included 16 interlaced PNGs and 8
progressive JPEGs. Reading all of them took about 35% longer than Windows does, which only matters at
full size, as shrinking is where this saves time.

## Animation

`cx.animate` moves a number smoothly, hover colors fade, scrolling glides, and popups fade in. It all
costs nothing when nothing's moving: there's no timer and no frame loop, as a frame only asks for
another while something's partway there. Only what's moving renders again each frame. Agents, tests and
anyone who's asked Windows not to animate get each result straight away.

Frames that show something are paced by the screen. Frames that turn out the same as the last aren't
shown, so nothing paces those, and they sleep for 8 ms instead. Asking the GPU to wait for the screen
isn't enough: `WaitForVBlank` came straight back on a machine with two GPUs, which meant 85,000 frames
in 3 seconds.

Measured in a real window on a 143 Hz screen, three seconds of motion took about 360 frames and 16 ms of
processor, and then none at all.

## Painting everything

A box with a background costs about 18 ns. Finding which part of
its look a prop sets takes one step, however many things drawing cares about, where it used to compare
with each. Everything most elements don't have sits behind one check, and rounding doesn't call a library.

| What | Before these | Now |
| --- | --- | --- |
| `paint/boxes_10k` | 318 µs | 183 µs |
| `paint/labels_2k` | 84 µs | 80 µs |
| `paint/dashboard` | 46 µs | 46 µs |
| `paint/paragraph_100k` | 199 µs | 200 µs |

Compare numbers with the settings that ship, from worktrees outside the repository, taking turns:
`CARGO_PROFILE_BENCH_LTO=fat CARGO_PROFILE_BENCH_CODEGEN_UNITS=1`. The bench profile builds faster, but its
16 codegen units can stop things being inlined because of code nearby, which looks like a slowdown of up to
30% when nothing's slower. An editor checking the code after each change adds noise to whatever's in its folder.

## More images

WebP comes out value for value as libwebp reads it: 822 files of every kind, with every number of partitions
and segments and both kinds of smoothing, and 48 animations. Arithmetic
JPEGs come out the same as the same files without it, in all 336 tried, and 12-bit ones are within 2 of
libjpeg-turbo. Animated PNGs are within 2 of what their spec's formula says, over 912 frames. Pillow's own
reading differs from that formula for see-through frames over see-through frames, so it isn't the reference
there. Color profiles are within 1 of LittleCMS, except for print colors, where 99% of values are within 3,
and the rest are colors no screen can show.

An animated image doesn't use a frame loop. Each frame asks for a timer, so one with ten frames a second
wakes ten times a second, and not at all while it's hidden or off screen. All its frames stay on the GPU, up
to 128 MB, at the size it's shown, as reading them again would cost far more than showing them.

## Text areas

A text input with several lines costs nothing extra until it's used, and working out where a caret goes
takes one pass over its lines, each time the caret moves or the text changes. Only the lines that show
are drawn.

## Popups

Menus, dialogs and tooltips go on top of everything else without costing anything when they're not
there: layout notes which boxes have one inside them, so finding them only follows those.

## Accessibility

Windows UI Automation works, so screen readers can read and use an app. A program pays nothing for this
until something asks: the system's library isn't loaded, no tree is copied, and nothing is checked. Once
something has asked, the window keeps a plain copy of the tree up to date as things change, as a screen
reader can ask from any thread at any time.

## Rendering

Open the Rust gallery in a real window, in release, on the same RTX 5070. The window is
944x601.

| Moment | Working set | Committed | GPU | Threads | Idle CPU |
|---|---|---|---|---|---|
| Just used | 4–8 MB | 50 MB | 16.3 MB | 25 | about 1 Mcycle/s |
| Quiet for a moment | 0.4 MB | 50 MB | 14.0 MB | 25 | about 1 Mcycle/s |
| Deep idle, still on screen | 0.2 MB | 10–20 MB | none | 6 | none |
| Minimized for 10 seconds | 0.1 MB | 10–20 MB | none | 3–5 | none |

Frames are measured with the process's cycle count, as its time is only counted in steps
of 15.6 ms. A frame of a typical window costs about
0.2 ms of CPU at most, and about 120 us on the GPU, against a floor of about 115 us to show anything.

What each of these does, and what it was worth:

- **Asking for a frame instead of drawing one:** input now lands in the next frame, however fast it
  arrives. 300 typed characters took 2,131 ms, and now take 49 ms, of which 32 ms is the window closing.
- **Starting Direct3D on another thread:** it takes about 100 ms, up to 170 ms the first time, and nearly
  all of it is inside the driver's device creation. The window comes up meanwhile, and reaches the
  screen with content in about 40 ms.
- **Deep idle:** the driver's threads wake even with nothing to draw. A quiet window hands its picture
  to the system and lets go of Direct3D. Across 4,438 screenshots through the handover, no pixel
  changed. Getting it back costs about 450 Mcycles, on another thread, and a click takes about 130 ms to
  show, so this only saves CPU after about 7 minutes. The wait backs off for windows in regular use.
  What it always saves is GPU memory, driver threads and commit charge.
- **Drawing only what changed:** frames compare their quads with the last frame's, and draw and report
  only the difference. A frame that changed nothing skips the GPU. This takes 10 to 20% off GPU time on
  this fast GPU, and costs about 10 us of CPU per frame. A slower GPU or a larger window should gain
  more, which hasn't been measured. After many partial frames, a real window matched a full redraw
  exactly, as do random scenes on a simulated swap chain.
- **One buffer while idle:** the second back buffer only exists after a second frame, so resizing to the
  same size and drawing once lets go of it. This saves one window's worth: 2.3 MB here, 33 MB at 4K. The
  rest of the GPU memory is the driver's, at 7.5 MB for having a device at all. Buffers and the atlas
  come out of pools that it has already allocated.
- **Trimming once early:** starting leaves a lot of memory in use that nothing touches again. Without
  this, an app that's always updating stayed at 39 MB, and now stays at 4.8 MB.

What didn't help:

| Idea | Result |
|---|---|
| A cache of recent glyphs in front of the atlas | 48.5 to 47.3 us a frame, within noise, for 6 KB. Removed |
| A waitable swap chain, so waiting is sleeping | No measurable change. Removed |
| OpenGL or Direct3D 9Ex | 8 to 10 threads rather than 25, and less waking, but 70% or more extra CPU per frame |
| Device settings or feature levels | 25 to 28 threads whatever they are |

Measuring real windows has traps. A person using one changes the results without any error, and that
happened repeatedly here, so check screenshots against what a script actually did.

### Painting

Measure with `cargo bench -p rsui --bench frame`, which needs no window.

| Operation | Before | Now |
|---|---|---|
| Paragraph of 100k characters, per frame | 3,628 us | 205 us |
| 10,000 boxes, per frame | 630 us | 340 us |
| Typical window, per frame | 73 us | 48 us |
| A key in a text input holding 1 MB | 48 ms | 9.7 ms |
| A key in a text input holding 10 KB | 476 us | 99 us |

What made the difference: reading each element's props in one pass by comparing pointers, and reusing
that for neighbors that share a style, rather than searching strings about 10 times per element. Text
that fits on a line takes one pass, not two. Long text remembers where its lines break and only places
what's on screen. Undo holds what each change replaced rather than copies of the value.

Nesting goes about 1,000 levels deep on a normal 1 MB stack in release, and overflows it before 2,000.

### Input

What makes input cheap is what it doesn't do:

- **Hover and pressed colors:** these are drawn from the element's props, with no state or component
  rendering. Moving the pointer only hit-tests, and draws nothing unless the hovered node changes.
- **Text inputs:** only the focused one has a caret and undo history. The rest are a plain element,
  so a form with many inputs costs the same as one with labels.
- **Typing:** each key goes through the same `SetValue` action an agent would use. The input's size
  doesn't depend on its value, so a test checks that typing lays out at most one node.

### What a covered window costs

Something moving in a window nobody can see draws 4 times a second until it can be seen. There's no
reliable report of this from the system, so `nothing_of_it_shows` in `window.rs` looks at the windows above,
only while something's moving and the window isn't in front. It only says yes for a window on another virtual
desktop or if one window covers it all, as something that can be seen must never slow down. With the cycle
counter, an animated app that renders 500 rows a frame went from 342 to 26 Mcycles a second while covered,
and a small one from 78 to 9. Uncovered, they're the same as they were, and it takes 250 ms at most to get
back to full speed unless the window got the keyboard, which is at once.

### Where the rest of the memory is

On the gallery, about 200 KB of heap is live once it's started, and 2.8 MB at most while it's reading an
image. Everything else in a window's working set is the driver and Windows, so there's no more to find by
looking at rsui's own allocations. Direct3D is now loaded on the thread that makes the renderer, which took
0.65 MB off what's in RAM before a window opens. `oleaut32.dll` and `combase.dll` would be the next thing to
load later, but the system's text input libraries load them anyway, so that wouldn't change anything.

Not worth doing, or not yet:

| Idea | Why not |
|---|---|
| Only moving what's inside when something scrolls, not laying it out | A notch is 24 us with 100 rows and 90 us with 1,000, and getting the same limits from what's kept is easy to get subtly wrong |
| Not keeping the tree for accessibility up to date when nothing's listening | A client can read without listening, and it would need to wait on the window's thread to catch up |
| A larger minimum before collecting JavaScript's garbage | About 20% of re-rendering 1,000 rows, but it's memory for speed, and was lowered to save memory |

## Agent tools

The server and tool figures below come from `rsui-agent`.

`rsui-tsx --agent` on the gallery, as a client would use it, measured on 2026-11-07 in the release build, each number being one run:

| | |
|---|---|
| To the first answer | 121 ms |
| `ui_snapshot` of 57 lines, or `ui_find` | 0.2 ms, 0.1 ms |
| `ui_act` for a click that changes something, over 50 | 0.15 ms in the middle, 0.31 ms at worst |
| `ui_screenshot` at 960 by 640 | 39 ms the first time, as the software GPU starts, then 11 ms |
| That picture | 146 KB as a file, 195 KB in the message, from 2,458 KB before it was compressed |
| Left waiting | 0.000 Mcycles/s, as it's only waiting for the next line |
| In RAM | 1.12 MB once it's started, 3.70 MB after the clicks, and 0.60 MB after a picture |
| The most in RAM at any point | 14.7 MB starting, and 24.5 MB with the software GPU |

The whole thing, along with `--do` and compressing pictures, adds 342 KB to the program, which is why a packed app has it too: it's the same program.
A packed app doesn't take `--agent` or `--do`, as its command line is its own.

- **Outline against JSON:** on a 411-node settings screen, the outline is 18,191 bytes against
  27,235 for the JSON snapshot. That's 67% of the size, at 44 bytes per node. The path on each
  line is a large share of that, and it's what lets an agent act without counting positions.
- **A running server:** the todo example over stdio, holding 50 todos, used 0.8 MB of private
  memory and a 5.2 MB working set, from a 599 KB release binary. This has no window, and it isn't
  on `rsui-alloc` or trimmed.
- **Long results:** lists of changes stop at about 4 KB and `ui_find` at 50 matches, each with a
  note saying what was left out, so one action can't flood an agent's context.
- **Agent input:** action, role and key names from agents are compared without being stored. Stored
  names are never freed, so an agent making up names can't grow the server's memory.

## Speed

| Operation | Start | Now | Change |
|---|---|---|---|
| Rebuild the tree and diff it, with pool and shared props | 5,745 µs | 600 µs | 9.6× |
| Apply a list shuffle | 5,321 µs | 70 µs | 76× |
| Diff, reverse list | 929 µs | 225 µs | 4× |
| Diff, shuffle list | 1,196 µs | 397 µs | 3× |
| Diff, rebuilt tree | 412 µs | 167 µs | 2.5× |
| JSON snapshot, reused buffer | 1,557 µs | 624 µs | 2.5× |
| Diff, one change, subtrees reused | 0.47 µs | 0.38 µs | 1.2× |

About 340 µs of the rebuild is formatting names with `text!`, which is the benchmark's
own workload rather than tree work.

### What worked

- **`NodePool`:** recycles dead trees' allocations.
- **Compact nodes:** reading less memory also sped up diffing and building.
- **`Patch::Reorder`:** applies a large reorder in one pass. Above 64 moves it replaces
  individual moves. The two cost about the same at that limit.
- **Direct table for compact integer keys:** skips hashing, and halved the reverse diff.
- **Hand-written JSON:** faster than serde. A test keeps its output byte-identical.
- **Inline paths:** most patches no longer allocate.
- **Run check in the LIS, linear Fenwick build:** nearly sorted lists no longer pay for a
  binary search per item. A far-apart swap went from 787 µs to 260 µs.

### What's worth doing outside this crate

- **`rsui-alloc` in applications:** building was about twice as fast as on the Windows heap. It
  has to be chosen by the application, so the final `rsui` crate should set it up.

### What didn't work

| Idea | Result | Why it was rejected |
|---|---|---|
| Custom 24-byte inline string type | 1.05 vs 1.57 ns per compare | Saves about 5 µs per 10,000 nodes, for 230 lines and `unsafe` |
| `lto = "fat"`, `codegen-units = 1`, `target-cpu=native` | Only snapshots improved, by about 15% | Costs live in memory, not in code generation |
| Deferred drop | Drop is about 15% of build | Small gain, and the pool removes most of the drop anyway |
| Inline storage for 2 props | About 7% on diff and build, none with the pool | Larger elements, and shared props do better |
| Comparing prop values before names | No measurable change | |

### Text made of parts, and shapes

Added on 2026-11-07, and compared with the commit before, with shipped settings in alternating rounds. Nothing that doesn't use them
changed by more than the noise between runs, which was up to 2.4% either way. The check for whether the pointer landed on a paragraph
comes to about 4 ns for each time it moves, in a program with none.

| What | Time |
|---|---|
| Painting a page of 30 paragraphs of plain text | 109 µs |
| The same with bold and italic words in each | 166 µs |
| Resizing the first | 159 µs |
| Resizing the second | 346 µs |

Paragraphs with parts place one word at a time, and nothing's remembered from one frame to the next. At these numbers it isn't worth
what remembering them would hold on to. A shape is drawn once at the size it's shown and kept as a picture on the GPU, and the same shape
in the same size and color many times is one picture. Each is one draw call, which would start to matter at some thousands of different
shapes on screen at once, and that hasn't been measured.

### More than one window

Measured on 2026-11-07, first on a desktop nobody could see, so that nothing came up on screen. A program with four windows, each with a little
text, against one with one.

| | One | Four |
|---|---|---|
| Most in RAM at any point | 19.0 MB | 22.5 to 23.9 MB |
| Most committed | 59 MB | 103 MB |

So a window costs 1 to 2 MB of RAM at its peak. It costs 15 MB of commit, which mostly isn't in RAM, as each has a Direct3D device of its own.
Sharing one would take that down, along with the time to open one, and it's the first thing to try if programs turn out to want a lot of windows.

Going idle can't be measured that way, as a window on a desktop nobody can see doesn't get there even when it's alone. So this is four real
windows on the real desktop, in the release build, left alone, with one reading of about a second each:

| After | Threads | In RAM | CPU |
|---|---|---|---|
| 2.6 s | 86 | 0.67 MB | 9.39 Mcycles/s |
| 13 s | 6 | 0.57 MB | 0.00 Mcycles/s |
| 21 s | 6 | 0.66 MB | 0.00 Mcycles/s |
| 31 s | 7 | 0.66 MB | 3.77 Mcycles/s |
| 41 s | 7 | 2.76 MB | 0.00 Mcycles/s |

They reach what one window does at about the same time: no driver, no CPU, and under a megabyte. The 31 s reading, one more thread and the 2.76 MB
aren't explained. Nothing in rsui is due then, as every window had let go of the GPU by 13 s, but it wasn't looked into further, and this was one run,
not compared with one window over the same time. A click in one window then brought all of them back, as what it changed shows in all of them: 88
threads, and a peak of 26.2 MB.

Giving memory back is for the whole program, so it waits until every window's been quiet. Each lets go of the GPU when it's been quiet itself,
and the driver only unloads once all of them have.

### What's still open

- **Flat tree stored in one array:** prototyped before the compact layout. It would drop the
  per-allocation overhead and the reference count, but it complicates sharing and moving
  subtrees. It needs real rebuild data from a runtime before it's worth the redesign.
- **Content hashes on nodes:** found one changed leaf in 0.64 µs, and hashing cost about
  13 ns per node. A rare collision would silently skip an update, so diffing would no longer
  be guaranteed correct. It's only worth revisiting for caching agent snapshots.
- **Parallel diff:** about 5× on a full rebuilt diff, on 16 threads. It only helps full
  rebuilds, where building costs more than diffing, and it adds overhead to small updates.
