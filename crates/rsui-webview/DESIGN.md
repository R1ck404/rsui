# rsui-webview

Web pages from elsewhere inside an rsui window. rsui draws an app's own UI itself, with no browser. Someone else's site is
written for a browser: its logins, captchas and checks expect Chromium. So `rsui/webview`'s `<WebView>` gets a real browser
view: WebView2, Edge's runtime, which comes with Windows 11. It's never used for the app's own UI, and it's only loaded when
the first view is put on screen.

## How it's put together

- **Finding the runtime** (`runtime.rs`). WebView2's own `WebView2Loader.dll` is a file an app ships beside itself, and all it
  does is read the registry and call into the runtime. This does the same: the stable runtime's folder is the `EBWebView`
  value under `HKLM` (or `HKCU`) `SOFTWARE\Microsoft\EdgeUpdate\ClientState\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`, read
  32-bit, and its `EBWebView\x64\EmbeddedBrowserWebView.dll` exports `CreateWebViewEnvironmentWithOptionsInternal`, which is
  what the loader calls, as the `webview` library's own loader does too. A `WebView2Loader.dll` beside the program is used
  instead if there is one, through its documented `CreateCoreWebView2EnvironmentWithOptions`, and
  `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` names a fixed-version runtime, as for WebView2's loader. The oldest runtime used is
  101.0.1210.39, which has profiles. Without one, a view says [`runtime::missing`]: what to install, and where from.
- **The COM interfaces** (`ffi.rs`) are written out by hand from `WebView2.h`, for what this calls, with
  `windows_core::interface`, as rsui writes its other COM. A method this never calls is only there to keep the rest in
  their places. Every handler WebView2 calls back is `IUnknown` and then `Invoke(a, b)`, so one small object
  (`handler.rs`) is all of them: it says it's whichever interface it was made as, and gives what it's called with to a
  closure. The `webview2-com` crate would have brought every WebView2 interface, generated from its metadata, on another
  version of the `windows` crate (0.61, where rsui has 0.62, so two of it would be built), and still needs
  `WebView2Loader.dll` shipped beside the program. What's here is about 600 lines and needs neither.
- **A view** (`view.rs`) is made the first time it's put somewhere, as WebView2 needs the window it's in. It sits in a window
  of its own, inside the app's, which is moved to what shows of its element and cut with a region where something's drawn
  on top of it; WebView2's own window inside that is put at the element's whole box. So what scrolls around it and what's
  drawn over it, such as a menu, cut it off as they would on a page. It only shows once its first page has started
  arriving, until when what's drawn under it, its address, shows. What's asked of it while it's being made is done once
  it's there, in order. Its environment, the browser processes, is one per data folder, shared by every view of it.
- **Profiles** ([`profile_name`]): a view's `profile` is a WebView2 profile of its own, with its own cookies and storage, and
  a `private` one is InPrivate, forgotten when it closes. They're all in one data folder, `WebView2` in the app's own local
  data folder, so they share browser processes. Nothing's ever in the person's own Edge.
- **Keys the app keeps** (`Settings::keys`): while a page has the keyboard, the keys an app names, such as a browser's
  Ctrl+T, are taken from WebView2's accelerator-key event and said as `Event::Key`, so an app's shortcuts work inside pages.

## Finding out why a view doesn't show

`RSUI_WEBVIEW_LOG=1` has each step said on the standard error, with what WebView2 gave back: the runtime that was found,
COM on the thread, asking for the environment and the view and what came of each, where the view's put, navigation, and
every event. A step that fails also reaches the app, as a failed load with -2 and the HRESULT in its description.

WebView2 can answer from inside the call that asks it, as runtime 157 does when it makes an environment, so what waits for
an answer is always there before it's asked.

## Where it's used from

- `rsui::embed` keeps something where an element with an `embed` number is, and tells it where that is each time the window
  is drawn: the element's box, what of it shows, what's drawn on top of it, and how round its corners are, in physical
  pixels. It costs nothing while nothing's kept.
- `rsui-tsx`'s `webview.rs` makes a view for each `<WebView>` that `webview.js` (`rsui/webview`) renders, in a window that's
  really running. With `--do`, `--agent` and `--png`, and in tests, it never makes one: what would load is said, and the
  `webview` steps say what the page did.

## What it costs

Nothing until it's used: no library is loaded, and the registry isn't read. When it's used, it's a browser: Edge's processes
for the data folder, which is about what a WebView2 app costs, shared by every view of it. `examples/measure.rs` measures
a view by hand: how long the first page took, and what the browser's processes hold.
