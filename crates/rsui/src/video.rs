//! Reading a video's frames, with the system's own decoders (Media Foundation on Windows): MP4, MOV, WebM, AVI, MKV and the rest
//! Windows plays, with nothing to install. Frames are taken at a steady rate, shrunk to a width, and given as RGBA, which is what
//! an app needs to use a short video as a picture that moves, not to play a film.

/// A video's frames, all the same size.
pub struct VideoFrames {
    pub width: u32,
    pub height: u32,
    /// Frames a second, as they were taken.
    pub fps: f32,
    /// Each frame's pixels, red, green, blue and alpha.
    pub frames: Vec<Vec<u8>>,
}

/// What a video's frames are taken as.
#[derive(Clone, Copy, Debug)]
pub struct VideoOptions {
    /// The most pixels across; a smaller video is kept as it is.
    pub max_width: u32,
    pub fps: f32,
    /// How much of the start is read.
    pub max_seconds: f32,
}

/// Shrinks BGRA (or BGRX) rows to RGBA at `width`, averaging what each pixel covers.
#[cfg(any(windows, test))]
fn shrink(
    src: &[u8],
    stride: usize,
    w: usize,
    h: usize,
    flip: bool,
    out_w: usize,
    out_h: usize,
) -> Vec<u8> {
    // Media Foundation normally negotiates the requested dimensions. In that case
    // the decoder already did the resampling and this is just a row swizzle.
    let row_bytes = w * 4;
    if w == out_w
        && h == out_h
        && stride >= row_bytes
        && w > 0
        && h > 0
        && (h - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(row_bytes))
            .is_some_and(|n| n <= src.len())
    {
        let mut out = vec![0; row_bytes * h];
        for (y, dst) in out.chunks_exact_mut(row_bytes).enumerate() {
            let at = (if flip { h - 1 - y } else { y }) * stride;
            for (target, source) in dst
                .chunks_exact_mut(4)
                .zip(src[at..at + row_bytes].chunks_exact(4))
            {
                target.copy_from_slice(&[source[2], source[1], source[0], 255]);
            }
        }
        return out;
    }
    let mut out = vec![0u8; out_w * out_h * 4];
    for oy in 0..out_h {
        let y0 = oy * h / out_h;
        let y1 = ((oy + 1) * h / out_h).max(y0 + 1).min(h);
        for ox in 0..out_w {
            let x0 = ox * w / out_w;
            let x1 = ((ox + 1) * w / out_w).max(x0 + 1).min(w);
            // A few samples each way is plenty for a picture this small.
            let sx = ((x1 - x0) / 3).max(1);
            let sy = ((y1 - y0) / 3).max(1);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            let mut y = y0;
            while y < y1 {
                let row = if flip { h - 1 - y } else { y };
                let mut x = x0;
                while x < x1 {
                    let k = row * stride + x * 4;
                    if k + 2 < src.len() {
                        b += u32::from(src[k]);
                        g += u32::from(src[k + 1]);
                        r += u32::from(src[k + 2]);
                        n += 1;
                    }
                    x += sx;
                }
                y += sy;
            }
            let o = (oy * out_w + ox) * 4;
            let n = n.max(1);
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
            out[o + 3] = 255;
        }
    }
    out
}

#[cfg(windows)]
pub fn decode(path: &std::path::Path, options: VideoOptions) -> Result<VideoFrames, String> {
    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, IMFMediaType, IMFSample, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
        MF_MT_INTERLACE_MODE, MF_MT_PIXEL_ASPECT_RATIO, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_ALL_STREAMS,
        MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MFVideoInterlace_Progressive, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
        MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Video,
        MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoFormat_RGB32,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::core::HSTRING;

    struct Session(bool);
    impl Drop for Session {
        fn drop(&mut self) {
            // SAFETY: balanced with the startup and initialisation below, on this thread.
            unsafe {
                let _ = MFShutdown();
                if self.0 {
                    CoUninitialize();
                }
            }
        }
    }

    let fail = |what: &str, e: windows::core::Error| format!("{what}: {}", e.message());
    // SAFETY: Media Foundation is started and stopped on this thread, and every object it gives is used only here.
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| fail("Media Foundation didn't start", e))?;
        let _session = Session(com);

        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        // A reader that gives RGB: with the system's full video processing it can also shrink the frames and use the graphics
        // card's decoder, which is far quicker than taking every frame at its own size; the plain one is kept for what that
        // can't do.
        let open = |advanced: bool| -> Result<(windows::Win32::Media::MediaFoundation::IMFSourceReader, IMFMediaType), String> {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&raw mut attributes, 2).map_err(|e| fail("no attributes", e))?;
            let attributes = attributes.ok_or("no attributes")?;
            if advanced {
                attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1).map_err(|e| fail("no video processing", e))?;
                let _ = attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
            } else {
                attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).map_err(|e| fail("no video processing", e))?;
            }
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attributes)
                .map_err(|_| "That video can’t be read, or Windows has no decoder for it".to_owned())?;
            let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
            reader.SetStreamSelection(video, true).map_err(|_| "That file has no video in it".to_owned())?;
            let wanted: IMFMediaType = MFCreateMediaType().map_err(|e| fail("no media type", e))?;
            wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| fail("media type", e))?;
            wanted.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(|e| fail("media type", e))?;
            if advanced {
                let native = reader.GetNativeMediaType(video, 0).map_err(|e| fail("media type", e))?;
                let size = native.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|e| fail("frame size", e))?;
                let (w, h) = ((size >> 32) as u64, size & 0xffff_ffff);
                if w == 0 || h == 0 {
                    return Err("That video has no size".to_owned());
                }
                let out_w = w.min(u64::from(options.max_width.max(2))) & !1;
                let out_h = ((h * out_w / w).max(2)) & !1;
                wanted.SetUINT64(&MF_MT_FRAME_SIZE, (out_w << 32) | out_h).map_err(|e| fail("frame size", e))?;
                let _ = wanted.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1);
                let _ = wanted.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32);
            }
            reader.SetCurrentMediaType(video, None, &wanted).map_err(|_| "That video can’t be turned into pictures".to_owned())?;
            let current = reader.GetCurrentMediaType(video).map_err(|e| fail("media type", e))?;
            Ok((reader, current))
        };
        let (reader, current) = match open(true) {
            Ok(opened) => opened,
            Err(_) => open(false)?,
        };
        let size = current.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|e| fail("frame size", e))?;
        let (w, h) = ((size >> 32) as usize, (size & 0xffff_ffff) as usize);
        if w == 0 || h == 0 {
            return Err("That video has no size".to_owned());
        }
        // A stride below zero is rows from the bottom up.
        let stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or((w * 4) as i32);
        let flip = stride < 0;
        let stride = stride.unsigned_abs() as usize;
        let out_w = w.min(options.max_width.max(1) as usize);
        let out_h = ((h * out_w) / w).max(1);

        let step = (10_000_000.0 / f64::from(options.fps.max(1.0))) as i64;
        let end = (f64::from(options.max_seconds) * 10_000_000.0) as i64;
        let mut next = 0i64;
        let mut frames = Vec::new();
        loop {
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            if reader.ReadSample(video, 0, None, Some(&raw mut flags), Some(&raw mut time), Some(&raw mut sample)).is_err() {
                break;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 || time > end {
                break;
            }
            let Some(sample) = sample else { continue };
            // A frame for each step of time: the decoder's own rate is thinned out to the one asked for.
            if time < next {
                continue;
            }
            next = next.max(time - step / 2) + step;
            let Ok(buffer) = sample.ConvertToContiguousBuffer() else { continue };
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut length = 0u32;
            if buffer.Lock(&raw mut data, None, Some(&raw mut length)).is_err() || data.is_null() {
                continue;
            }
            let bytes = std::slice::from_raw_parts(data, length as usize);
            frames.push(shrink(bytes, stride, w, h, flip, out_w, out_h));
            let _ = buffer.Unlock();
            if frames.len() > 2000 {
                break;
            }
        }
        if frames.is_empty() {
            return Err("No frames could be read from that video".to_owned());
        }
        Ok(VideoFrames { width: out_w as u32, height: out_h as u32, fps: options.fps, frames })
    }
}

#[cfg(not(windows))]
pub fn decode(_: &std::path::Path, _: VideoOptions) -> Result<VideoFrames, String> {
    Err("videos are only read on Windows so far".to_owned())
}

#[cfg(test)]
mod tests {
    use super::shrink;

    #[test]
    fn shrinking_averages_and_turns_bgr_into_rgb() {
        // Two by one, blue then red, as BGRX.
        let src = [255, 0, 0, 0, 0, 0, 255, 0];
        let out = shrink(&src, 8, 2, 1, false, 2, 1);
        assert_eq!(&out[0..4], &[0, 0, 255, 255]);
        assert_eq!(&out[4..8], &[255, 0, 0, 255]);
        let half = shrink(&src, 8, 2, 1, false, 1, 1);
        assert_eq!(&half[0..3], &[127, 0, 127]);
    }

    #[test]
    fn bottom_up_rows_come_out_the_right_way_up() {
        // One pixel wide, two tall, stored bottom row first.
        let src = [0, 0, 255, 0, 255, 0, 0, 0];
        let out = shrink(&src, 4, 1, 2, true, 1, 2);
        assert_eq!(&out[0..3], &[0, 0, 255]);
        assert_eq!(&out[4..7], &[255, 0, 0]);
    }
}
