# Social Batch Export

## Native UI

File → Automate → Social Batch Export… works even when no document is open and no action has been recorded. Fields are stored in the existing data-driven dialog system.

1. Source folder: a folder containing images supported by PhotoCraft.
2. Destination folder: created if needed; use a fresh folder for each rerun.
3. Size preset: Instagram square / portrait, story, or YouTube thumbnail.
4. Resize mode: crop (fill) or fit (contain and pad).
5. Format: PNG or JPEG.
6. JPEG quality: 0–12, default 10; ignored for PNG.
7. Fit background: `#RRGGBB`, default `#191919`.

The existing status-bar job UI provides file-count progress and cancellation. Completion posts a result notice; per-file failures make the status an error rather than a false success.

## CLI examples

```sh
photocraft-cli social-export --in photos --out square --preset instagram-post --mode crop
photocraft-cli social-export --in photos --out portrait --preset instagram-portrait --mode crop
photocraft-cli social-export --in photos --out story --preset story --mode fit --background '#ffffff'
photocraft-cli social-export --in photos --out thumbnails --preset youtube-thumbnail --format jpg --quality 10 --json
```

The report includes `files`, `errors`, `warnings`, `width`, `height`, `preset`, `succeeded`, and `failed`. Invalid options fail before image processing. A corrupt/missing file does not prevent other inputs from finishing. Any per-file failure makes the CLI exit nonzero.

## Data safety

- The source is imported into a scratch Session; the original and the caller's open documents are never edited.
- Aspect ratio is preserved to the nearest pixel. Crop resizes to cover; fit resizes to contain.
- Resizing and canvas operations reuse PhotoCraft's depth-aware engine at 8/16/32-bit depth.
- Fit colour uses the existing colour conversion path and a bottom fill for transparent documents.
- Encoded output is written to a temporary file in the destination directory, synchronised, then published with no-clobber persistence.
- Existing files and symlinks are refused. A duplicate destination name is an error, not replacement.
- Inputs are capped at 10,000; intermediate resampling is capped at 32 million pixels / 32,768 per side, in addition to upstream allocation safeguards.
- Cancellation is checked between files and processing stages. Already completed exports remain; a single resize or encode may finish before cancellation is observed.

## Architecture

`crates/engine/src/social_export_cmds.rs`: presets, strict parsing, resize planning, batch job, exclusive writes and engine tests.

`crates/ui-egui/src/menu_catalog.rs` + `view_cmds.rs`: menu, native dialog and result notices. `jobs_ui.rs` handles asynchronous completion.

`apps/photocraft-cli/src/lib.rs`: trusted local `social-export` subcommand and JSON/text reports.

No new third-party Rust dependency was required: tempfile is already a native engine dependency. No sandbox authorization exception was added.
