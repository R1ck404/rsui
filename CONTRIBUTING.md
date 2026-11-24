# Contributing

rsui is young and its APIs can still change, so for anything bigger than a fix it's worth opening an issue first to ask whether it
fits.

## Building

You need what the [README](README.md#getting-started) says: Windows 10 or later, and Rust 1.85 or later with the MSVC toolchain.
Node is only needed to run a demo, which gets its packages with `npm install`.

```
cargo build --release -p rsui-tsx
cargo check --workspace --all-targets --locked
```

Those two are what CI runs. If you add a dependency, commit the `Cargo.lock` change with it, since both use `--locked`.

## Testing

A crate with no window or GPU in it is tested with plain cargo:

```
cargo test -p rsui-core -p rsui-layout -p rsui-text -p rsui-image -p rsui-alloc -p rsui-js
```

The tests of `rsui` open real windows, on a desktop of their own, so nothing shows on screen and nothing takes your keyboard. Many
tests of `rsui-tsx` run apps through the same path as `--do`, so set `RSUI_SANDBOX` to an empty folder before running them, and
anything they would save to your own settings goes there instead.

Some tests are marked `#[ignore]`, and the attribute says why. Most are measurements to run by hand with `--ignored --nocapture`.
The rest need something the test machine may not have, such as Node or the sound device.

## Before sending a change

- `cargo clippy --workspace --all-targets` should show no new warnings. A few dozen style warnings are already there, and the
  lints that are allowed on purpose are in the root `Cargo.toml` with the reason.
- The code isn't run through `rustfmt`. It's wrapped by hand at about 120 columns, so match what's around your change.
- Comments here say why. Many record what was measured and what didn't work, so nobody has to try it again. Keep that going: if you
  tried something and dropped it, a line saying so is useful.
- A change made for speed or memory needs a measurement. Compare against a build of the base commit made with the same settings,
  as the build profile alone can move a result by more than the change does. Run each side more than once. `crates/rsui-core/PERFORMANCE.md`
  and the budgets in `crates/rsui-js/DESIGN.md` show what's measured and how.
- Apps are written as plain React, with no `memo` or `useCallback` for speed. If a plain app is slow, that's a bug in the engine.

## License

Unless you say otherwise, a contribution is dual licensed under the Apache License 2.0 and the MIT license, as the
[README](README.md#license) says.
