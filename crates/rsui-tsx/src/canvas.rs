//! What `canvas.js` calls: drawing what a page's canvas was told, reading its pixels back, and measuring text. The drawing itself
//! is `rsui::render::canvas`; this only hands it what the page's code gave. Nothing here runs until a page has a canvas.

use rsui::render::canvas;
use rsui_js::{Args, HostFunction, Value, Vm};

pub(crate) const FUNCTIONS: [(&str, HostFunction); 14] = [
    ("canvasCreate", host_create),
    ("canvasResize", host_resize),
    ("canvasRelease", host_release),
    ("canvasRun", host_run),
    ("canvasRead", host_read),
    ("canvasWrite", host_write),
    ("canvasMeasure", host_measure),
    ("canvasTextPath", host_text_path),
    ("canvasPointIn", host_point_in),
    ("canvasLoadImage", host_load_image),
    ("canvasSnapshot", host_snapshot),
    ("canvasReleaseImage", host_release_image),
    ("canvasEncode", host_encode),
    ("canvasDataUrl", host_data_url),
];

fn number(vm: &Vm, args: &Args, at: usize) -> f64 {
    args.get(vm, at).as_number()
}

fn whole(vm: &Vm, args: &Args, at: usize) -> u32 {
    let n = number(vm, args, at);
    if n.is_finite() && n > 0.0 { n as u32 } else { 0 }
}

/// `canvasCreate(width, height, opaque)`: a new canvas's number.
fn host_create(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let opaque = args.get(vm, 2) == Value::TRUE;
    let id = canvas::create(whole(vm, &args, 0), whole(vm, &args, 1), opaque);
    Ok(Value::number(f64::from(id)))
}

/// `canvasResize(id, width, height)`: empties it at this size, and says whether it could be that big.
fn host_resize(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let fits = canvas::resize(whole(vm, &args, 0), whole(vm, &args, 1), whole(vm, &args, 2));
    Ok(Value::bool(fits))
}

fn host_release(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    canvas::release(whole(vm, &args, 0));
    Ok(Value::UNDEFINED)
}

/// `canvasRun(list, count, strings)`: draws the first `count` numbers of a `Float64Array`, with `strings` for text.
fn host_run(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let list = args.get(vm, 0);
    let count = whole(vm, &args, 1) as usize;
    let strings_value = args.get(vm, 2);
    let strings: Vec<String> = vm
        .array_elements(strings_value)
        .map(|items| items.iter().map(|item| vm.string_text(*item).map(|t| t.into_owned()).unwrap_or_default()).collect())
        .unwrap_or_default();
    let Some(bytes) = vm.bytes(list) else {
        return Ok(Value::UNDEFINED);
    };
    let count = count.min(bytes.len() / 8);
    let bytes = &bytes[..count * 8];
    // Read where they are when they're lined up as numbers are, which they nearly always are.
    if (bytes.as_ptr() as usize) % std::mem::align_of::<f64>() == 0 {
        // SAFETY: the bytes are lined up for `f64`, `count` of them fit, and any bits are a number.
        let numbers = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<f64>(), count) };
        canvas::run(numbers, &strings);
    } else {
        let numbers: Vec<f64> = bytes.chunks_exact(8).map(|eight| f64::from_le_bytes(eight.try_into().expect("eight"))).collect();
        canvas::run(&numbers, &strings);
    }
    Ok(Value::UNDEFINED)
}

/// `canvasRead(id, x, y, width, height, into)`: fills a `Uint8ClampedArray` as `getImageData()` gives it.
fn host_read(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = whole(vm, &args, 0);
    let (x, y) = (number(vm, &args, 1) as i32, number(vm, &args, 2) as i32);
    let (w, h) = (whole(vm, &args, 3), whole(vm, &args, 4));
    let into = args.get(vm, 5);
    if let Some(out) = vm.bytes_mut(into) {
        canvas::read(id, x, y, w, h, out);
    }
    Ok(Value::UNDEFINED)
}

/// `canvasWrite(id, data, dataWidth, dataHeight, dx, dy, dirtyX, dirtyY, dirtyWidth, dirtyHeight)`, as `putImageData()`.
fn host_write(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = whole(vm, &args, 0);
    let data = args.get(vm, 1);
    let (dw, dh) = (whole(vm, &args, 2), whole(vm, &args, 3));
    let at = |i: usize| {
        let n = args.get(vm, i).as_number();
        if n.is_finite() { n.clamp(-1.0e9, 1.0e9) as i32 } else { 0 }
    };
    let (dx, dy) = (at(4), at(5));
    let dirty = [at(6), at(7), at(8), at(9)];
    if let Some(bytes) = vm.bytes(data) {
        canvas::write(id, bytes, dw, dh, dx, dy, dirty);
    }
    Ok(Value::UNDEFINED)
}

/// `canvasMeasure(size, weight, italic, family, letterSpacing, rtl, align, baseline, text)`: what `measureText()` gives, as 12
/// numbers.
fn host_measure(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let font = [number(vm, &args, 0), number(vm, &args, 1), number(vm, &args, 2)];
    let family = args.get(vm, 3);
    let family = vm.string_text(family).map(|t| t.into_owned()).unwrap_or_default();
    let spacing = number(vm, &args, 4) as f32;
    let rtl = args.get(vm, 5) == Value::TRUE;
    let (align, baseline) = (whole(vm, &args, 6) as u8, whole(vm, &args, 7) as u8);
    let text = args.get(vm, 8);
    let text = vm.string_text(text).map(|t| t.into_owned()).unwrap_or_default();
    let found = canvas::measure(&font, &family, if spacing.is_finite() { spacing } else { 0.0 }, rtl, align, baseline, &text);
    let items = found.iter().map(|n| Value::number(*n)).collect();
    Ok(vm.new_array(items))
}

/// `canvasTextPath(size, weight, italic, family, letterSpacing, text)`: the text as an SVG path, in pixels from where its pen
/// starts on its baseline, and how wide it is, as `[d, width]`, for an `<svg>`'s `<text>`.
fn host_text_path(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let font = [number(vm, &args, 0), number(vm, &args, 1), number(vm, &args, 2)];
    let family = args.get(vm, 3);
    let family = vm.string_text(family).map(|t| t.into_owned()).unwrap_or_default();
    let spacing = number(vm, &args, 4) as f32;
    let text = args.get(vm, 5);
    let text = vm.string_text(text).map(|t| t.into_owned()).unwrap_or_default();
    let (d, width) = canvas::text_path(&font, &family, if spacing.is_finite() { spacing } else { 0.0 }, &text);
    let d = vm.new_string(&d);
    Ok(vm.new_array(vec![d, Value::number(f64::from(width))]))
}

/// `canvasPointIn(id, path, x, y, evenOdd, stroke)`.
fn host_point_in(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = whole(vm, &args, 0);
    let path = whole(vm, &args, 1);
    let (x, y) = (number(vm, &args, 2) as f32, number(vm, &args, 3) as f32);
    let even_odd = args.get(vm, 4) == Value::TRUE;
    let stroke = args.get(vm, 5) == Value::TRUE;
    if !(x.is_finite() && y.is_finite()) {
        return Ok(Value::FALSE);
    }
    Ok(Value::bool(canvas::point_in(id, path, x, y, even_odd, stroke)))
}

/// `canvasLoadImage(id, source)`: reads a picture for `drawImage()`, and says its size, or nothing if it can't be read.
fn host_load_image(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = whole(vm, &args, 0);
    let source = args.get(vm, 1);
    let Some(source) = vm.plain_text(source) else {
        return Ok(Value::UNDEFINED);
    };
    Ok(match canvas::load_image(id, &source) {
        Some((w, h)) => {
            let items = vec![Value::number(f64::from(w)), Value::number(f64::from(h))];
            vm.new_array(items)
        }
        None => Value::UNDEFINED,
    })
}

/// `canvasSnapshot(canvas, id)`: keeps what a canvas shows as a picture, and says its size.
fn host_snapshot(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    Ok(match canvas::snapshot(whole(vm, &args, 0), whole(vm, &args, 1)) {
        Some((w, h)) => {
            let items = vec![Value::number(f64::from(w)), Value::number(f64::from(h))];
            vm.new_array(items)
        }
        None => Value::UNDEFINED,
    })
}

fn host_release_image(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    canvas::release_image(whole(vm, &args, 0));
    Ok(Value::UNDEFINED)
}

/// A canvas as a PNG file, or nothing for one with no pixels.
fn png_of(id: u32) -> Option<Vec<u8>> {
    let (w, h) = canvas::size(id)?;
    if w == 0 || h == 0 {
        return None;
    }
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    canvas::read(id, 0, 0, w, h, &mut pixels);
    Some(crate::png::encode(w as usize, h as usize, &pixels))
}

/// `canvasEncode(id)`: an `ArrayBuffer` of a PNG file of it, for `toBlob()`, or nothing.
fn host_encode(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    Ok(match png_of(whole(vm, &args, 0)) {
        Some(bytes) => vm.new_buffer(bytes),
        None => Value::NULL,
    })
}

/// `canvasDataUrl(id)`: a `data:` URL of a PNG file of it, as `toDataURL()` gives, which is `data:,` for one with no pixels.
fn host_data_url(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = match png_of(whole(vm, &args, 0)) {
        Some(bytes) => {
            let mut out = String::with_capacity(22 + bytes.len() * 4 / 3 + 4);
            out.push_str("data:image/png;base64,");
            base64_into(&bytes, &mut out);
            out
        }
        None => "data:,".to_owned(),
    };
    Ok(vm.new_string(&text))
}

fn base64_into(bytes: &[u8], out: &mut String) {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for three in bytes.chunks(3) {
        let n = u32::from(three[0]) << 16 | u32::from(*three.get(1).unwrap_or(&0)) << 8 | u32::from(*three.get(2).unwrap_or(&0));
        out.push(LETTERS[(n >> 18) as usize & 63] as char);
        out.push(LETTERS[(n >> 12) as usize & 63] as char);
        out.push(if three.len() > 1 { LETTERS[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if three.len() > 2 { LETTERS[n as usize & 63] as char } else { '=' });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_pads_as_it_should() {
        let mut out = String::new();
        super::base64_into(b"hi!", &mut out);
        assert_eq!(out, "aGkh");
        out.clear();
        super::base64_into(b"hi", &mut out);
        assert_eq!(out, "aGk=");
    }
}
