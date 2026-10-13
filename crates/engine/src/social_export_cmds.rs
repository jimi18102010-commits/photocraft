//! Non-destructive, native social-media batch export. Scratch sessions preserve the caller's
//! open documents; exclusive output creation never replaces an existing result or original.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{SaveOpts, encode, file_name, import, join, native, read_file, stem, str_param};
use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session};

pub const COMMAND: &str = "file.automate.socialExport";
pub const PRESETS: &[(&str, u32, u32)] =
    &[("instagram-post", 1080, 1080), ("instagram-portrait", 1080, 1350), ("story", 1080, 1920), ("youtube-thumbnail", 1280, 720)];

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}

/// Integer output sizes preserve aspect ratio to the nearest pixel, without stretching.
/// Crop covers the canvas; fit contains the image and pads the remainder.
pub fn resize_plan(source: (u32, u32), target: (u32, u32), crop: bool) -> Result<(u32, u32)> {
    let (sw, sh) = source;
    let (tw, th) = target;
    if sw == 0 || sh == 0 || tw == 0 || th == 0 {
        return Err(bad("image dimensions must be positive"));
    }
    let x = f64::from(tw) / f64::from(sw);
    let y = f64::from(th) / f64::from(sh);
    let scale = if crop { x.max(y) } else { x.min(y) };
    let round = |v: f64| if crop { v.ceil() } else { v.round().max(1.0) };
    let w = round(f64::from(sw) * scale);
    let h = round(f64::from(sh) * scale);
    // Refuse pathological aspect ratios before any resampling allocation.
    if w > 32_768.0 || h > 32_768.0 || w * h > 32_000_000.0 {
        return Err(bad("crop would create an oversized intermediate image; use fit instead"));
    }
    Ok((w as u32, h as u32))
}

#[derive(Clone)]
struct Options {
    inputs: Vec<String>,
    output: String,
    preset: String,
    target: (u32, u32),
    crop: bool,
    format: String,
    background: String,
    save: SaveOpts,
}

fn string_choice<'a>(p: &'a Value, key: &str, default: &'a str, allowed: &[&str]) -> Result<&'a str> {
    let value = match p.get(key) {
        None => default,
        Some(v) => v.as_str().ok_or_else(|| bad(format!("{key} must be a string")))?,
    };
    if !allowed.contains(&value) {
        return Err(bad(format!("unknown {key} `{value}`; choose {}", allowed.join(", "))));
    }
    Ok(value)
}

impl Options {
    fn parse(s: &Session, p: &Value) -> Result<Self> {
        let preset = string_choice(p, "preset", "instagram-post", &["instagram-post", "instagram-portrait", "story", "youtube-thumbnail"])?;
        let target = PRESETS.iter().find(|(id, _, _)| *id == preset).map(|(_, w, h)| (*w, *h)).ok_or_else(|| bad("unknown preset"))?;
        let crop = string_choice(p, "mode", "crop", &["crop", "fit"])? == "crop";
        let format = string_choice(p, "format", "png", &["png", "jpg"])?;
        let output = str_param(p, "output", COMMAND)?.to_owned();
        let background = match p.get("background") {
            None => "#191919",
            Some(v) => v.as_str().ok_or_else(|| bad("background must be #RRGGBB"))?,
        };
        if background.len() != 7 || !background.starts_with('#') || !background.bytes().skip(1).all(|b| b.is_ascii_hexdigit()) {
            return Err(bad("background must be #RRGGBB"));
        }
        if let Some(q) = p.get("quality") {
            q.as_f64().filter(|q| q.is_finite() && (0.0..=12.0).contains(q)).ok_or_else(|| bad("quality must be a number from 0 to 12"))?;
        }
        let inputs = match p.get("input") {
            Some(Value::Array(a)) => {
                if a.len() > 10_000 {
                    return Err(bad("at most 10000 input files are allowed"));
                }
                a.iter()
                    .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned).ok_or_else(|| bad("every input must be a non-empty file path")))
                    .collect::<Result<Vec<_>>>()?
            }
            Some(Value::String(dir)) if !dir.is_empty() => crate::file_cmds::list_images(dir)?,
            _ => return Err(bad("input must be a folder path or an array of files")),
        };
        if inputs.is_empty() || inputs.len() > 10_000 {
            return Err(bad("select between 1 and 10000 images"));
        }
        Ok(Self {
            inputs,
            output,
            preset: preset.into(),
            target,
            crop,
            format: format.into(),
            background: background.into(),
            save: SaveOpts::from_params(s, p, COMMAND)?.or_quality(10.0),
        })
    }
}

fn prepare(scratch: &mut Session, target: (u32, u32), crop: bool, background: &str) -> Result<()> {
    let d = scratch.active().ok_or(EngineError::NoDocument)?;
    let resized = resize_plan((d.doc.size.width, d.doc.size.height), target, crop)?;
    scratch.execute("image.imageSize", json!({"width": resized.0, "height": resized.1, "resample": "bicubic"}))?;
    scratch.execute("image.canvasSize", json!({"width": target.0, "height": target.1, "anchor": "center", "extensionColor": background}))?;
    // Canvas Size extends a locked Background, but transparent documents have no such layer.
    // Add a colour-managed fill behind those images so fit always honours its background.
    if !crop && scratch.active().is_some_and(|st| st.doc.background().is_none()) {
        let rgba = crate::commands::color_param(&json!({"color": background}), "color", [0.0, 0.0, 0.0, 1.0]);
        scratch.edit("Fit Background", |doc, _| {
            let fill = photocraft_doc::Fill::Solid(photocraft_color::Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3])).in_mode(doc.mode);
            let layer = photocraft_doc::Layer::new("Social Export Background", photocraft_doc::LayerContent::Fill(fill));
            doc.layers.insert(0, layer);
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn write_new(path: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let path = std::path::Path::new(path);
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| std::path::Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| EngineError::Other(e.to_string()))?;
    file.write_all(bytes).and_then(|()| file.as_file().sync_all()).map_err(|e| EngineError::Other(e.to_string()))?;
    file.persist_noclobber(path)
        .map_err(|e| EngineError::Other(format!("not written: {}: {} (existing outputs are never overwritten)", path.display(), e.error)))?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn write_new(_path: &str, _bytes: &[u8]) -> Result<()> {
    Err(EngineError::Other("social export requires the native app".into()))
}

fn process(options: &Options, ctx: &JobCtx) -> Result<Value> {
    ctx.check()?;
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::create_dir_all(&options.output).map_err(|e| EngineError::Other(format!("{}: {e}", options.output)))?;
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for (index, input) in options.inputs.iter().enumerate() {
        ctx.check()?;
        ctx.progress(index as f32 / options.inputs.len() as f32, &format!("{} / {} · {}", index + 1, options.inputs.len(), file_name(input)));
        let out = join(&options.output, &format!("{}-{}.{}", stem(input), options.preset, options.format));
        let result = (|| -> Result<Vec<String>> {
            // Fail early for existing files; write_new is still the race-safe final gate.
            #[cfg(not(target_arch = "wasm32"))]
            if std::fs::symlink_metadata(&out).is_ok() {
                return Err(EngineError::Other(format!("not written: {out} already exists")));
            }
            let bytes = read_file(input)?;
            let doc = import(&file_name(input), &bytes)?;
            ctx.check()?;
            let mut scratch = Session::new();
            scratch.add_document(doc, None);
            prepare(&mut scratch, options.target, options.crop, &options.background)?;
            ctx.check()?;
            let d = scratch.active().ok_or(EngineError::NoDocument)?;
            let (bytes, warnings) = encode(&d.doc, &out, options.save)?;
            ctx.check()?;
            write_new(&out, &bytes)?;
            Ok(warnings)
        })();
        match result {
            Ok(ws) => {
                files.push(out);
                warnings.extend(ws.into_iter().map(|warning| json!({"file": input, "warning": warning})));
            }
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(e) => errors.push(json!({"file": input, "error": e.to_string()})),
        }
        ctx.progress((index + 1) as f32 / options.inputs.len() as f32, "");
    }
    Ok(json!({"files": files, "errors": errors, "warnings": warnings, "preset": options.preset,
        "width": options.target.0, "height": options.target.1, "succeeded": files.len(), "failed": errors.len()}))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let options = Options::parse(s, p)?;
    crate::jobs::run(s, "Social Batch Export", false, move |ctx| process(&options, ctx), |_, result| Ok(result))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "Social Batch Export…",
        menu: &["File", "Automate"],
        shortcut: None,
        params: r##"{"input":folder|[paths],"output":folder,"preset":"instagram-post|instagram-portrait|story|youtube-thumbnail"="instagram-post","mode":"crop|fit"="crop","format":"png|jpg"="png","quality":0..12=10,"background":"#RRGGBB"="#191919"} → {files,errors,warnings,preset,width,height,succeeded,failed}; existing outputs never overwritten; native only"##,
        enabled: native,
        run,
        journal: true,
    }]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use photocraft_doc::Size;

    fn fixture(dir: &std::path::Path, name: &str) -> Result<String> {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 16, "background": "#2783de"}))?;
        let path = dir.join(name).to_string_lossy().into_owned();
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        crate::file_cmds::save_doc(doc, &path, None)?;
        Ok(path)
    }

    #[test]
    fn aspect_ratio_is_preserved_and_crop_covers() {
        assert_eq!(resize_plan((400, 200), (100, 100), true).unwrap(), (200, 100));
        assert_eq!(resize_plan((400, 200), (100, 100), false).unwrap(), (100, 50));
        assert_eq!(resize_plan((200, 400), (100, 100), false).unwrap(), (50, 100));
        assert!(resize_plan((0, 2), (100, 100), false).is_err());
        assert!(resize_plan((1, u32::MAX), (1080, 1080), true).is_err());
    }

    #[test]
    fn fit_and_crop_work_at_every_depth_without_mutating_the_source() {
        for depth in [8, 16, 32] {
            for crop in [false, true] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 20, "height": 10, "depth": depth, "background": "#2783de"})).unwrap();
                let original = s.active().unwrap().doc.clone();
                prepare(&mut s, (12, 12), crop, "#191919").unwrap();
                assert_eq!(s.active().unwrap().doc.size, Size::new(12, 12));
                assert_eq!(s.active().unwrap().doc.depth, original.depth);
                assert_eq!(original.size, Size::new(20, 10));
            }
        }
    }

    #[test]
    fn every_preset_produces_exact_canvas_dimensions() {
        for (_, w, h) in PRESETS {
            for crop in [false, true] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 20, "height": 10, "background": "#2783de"})).unwrap();
                prepare(&mut s, (*w, *h), crop, "#191919").unwrap();
                assert_eq!(s.active().unwrap().doc.size, Size::new(*w, *h));
            }
        }
    }

    #[test]
    fn transparent_fit_uses_the_chosen_background() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": "#2783de"})).unwrap();
        assert!(s.active().unwrap().doc.background().is_none());
        prepare(&mut s, (12, 12), false, "#ff0000").unwrap();
        let d = &s.active().unwrap().doc;
        let pixel = photocraft_compose::render(d, photocraft_geom::Rect::new(0, 0, 1, 1)).px[0];
        assert!(pixel[0] > 0.99 && pixel[1] < 0.01 && pixel[2] < 0.01 && pixel[3] > 0.99, "{pixel:?}");
    }

    #[test]
    fn invalid_parameters_fail_before_writes() {
        let s = Session::new();
        for p in [
            json!({}),
            json!({"input": [], "output": "unused"}),
            json!({"input": [42], "output": "unused"}),
            json!({"input": ["a.png"], "output": "unused", "preset": "fake"}),
            json!({"input": ["a.png"], "output": "unused", "mode": "stretch"}),
            json!({"input": ["a.png"], "output": "unused", "format": "exe"}),
            json!({"input": ["a.png"], "output": "unused", "quality": 13}),
            json!({"input": ["a.png"], "output": "unused", "background": "red"}),
            json!({"input": ["a.png"], "output": "unused", "quality": "bad"}),
        ] {
            assert!(Options::parse(&s, &p).is_err(), "{p}");
        }
    }

    #[test]
    fn batch_exports_exact_dimensions_and_collects_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let input = fixture(tmp.path(), "source.png").unwrap();
        let before = std::fs::read(&input).unwrap();
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 7, "height": 9})).unwrap();
        let r = s
            .execute(COMMAND, json!({"input": [input, "missing.png"], "output": tmp.path().join("out"), "preset": "youtube-thumbnail", "mode": "fit"}))
            .unwrap();
        assert_eq!(r["succeeded"], 1);
        assert_eq!(r["failed"], 1);
        let bytes = std::fs::read(r["files"][0].as_str().unwrap()).unwrap();
        let exported = import("output.png", &bytes).unwrap();
        assert_eq!(exported.size, Size::new(1280, 720));
        assert_eq!(std::fs::read(&input).unwrap(), before);
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.active().unwrap().doc.size, Size::new(7, 9));
    }

    #[test]
    fn existing_outputs_and_name_collisions_are_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let input = fixture(tmp.path(), "a.png").unwrap();
        let out = tmp.path().join("a-instagram-post.png");
        std::fs::write(&out, b"keep me").unwrap();
        let mut s = Session::new();
        let r = s.execute(COMMAND, json!({"input": [input], "output": tmp.path()})).unwrap();
        assert_eq!(r["failed"], 1);
        assert_eq!(std::fs::read(out).unwrap(), b"keep me");
        let a = tmp.path().join("exclusive.png");
        write_new(a.to_str().unwrap(), b"first").unwrap();
        assert!(write_new(a.to_str().unwrap(), b"second").is_err());
        assert_eq!(std::fs::read(a).unwrap(), b"first");
    }

    #[test]
    fn cancelled_batch_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let input = fixture(tmp.path(), "a.png").unwrap();
        let options = Options::parse(&Session::new(), &json!({"input": [input], "output": tmp.path().join("out")})).unwrap();
        let ctx = JobCtx::new();
        ctx.cancel();
        assert!(matches!(process(&options, &ctx), Err(EngineError::Cancelled)));
        assert!(!tmp.path().join("out").exists());
    }
}
