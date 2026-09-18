# `icon/`

**Everything here is generated. Do not edit these files.**

```sh
cargo run -p km-display --example icon
```

That writes this directory and the Android and iOS launcher resources too; the design lives in
`crates/playback/km-display/examples/icon.rs`, and the colors come from `km_display::theme::Theme`.

**Deliberately not under `assets/`**, which is shipped beside the binary: nothing in `icon/` is read
at run time.

The palette, the seven theme fields it may use, and what a fourth program cost are in
[`README.md`](README.md).
