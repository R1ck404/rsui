//! `video` in `rsui/desktop`: a video's frames, read with the system's decoders on a thread of their own and kept here, so a frame
//! goes to JavaScript only when it's asked for, copied into a buffer it already has. It's only read, so it's real while an app's
//! being checked too.

use std::collections::HashMap;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};

use rsui::video::{VideoFrames, VideoOptions};
use rsui_js::{Args, HostFunction, Value, Vm};

use crate::native::{number_at, text_at};

type Videos = Arc<Mutex<HashMap<u32, VideoFrames>>>;

thread_local! {
    static VIDEOS: RefCell<Option<Videos>> = const { RefCell::new(None) };
}
static NEXT: AtomicU32 = AtomicU32::new(1);

pub(crate) fn let_go_of_everything() {
    VIDEOS.with_borrow_mut(Option::take);
}

/// `videoOpen(path, maxWidth, fps, seconds)`: a promise of `{ id, width, height, fps, count }` as JSON.
fn host_open(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let path = text_at(vm, args, 0);
    let max_width = number_at(vm, args, 1).clamp(16, 1920);
    let fps = args.get(vm, 2).as_number();
    let seconds = args.get(vm, 3).as_number();
    let options = VideoOptions {
        max_width,
        fps: if fps.is_finite() && fps > 0.0 { fps.min(60.0) as f32 } else { 12.0 },
        max_seconds: if seconds.is_finite() && seconds > 0.0 { seconds.min(120.0) as f32 } else { 15.0 },
    };
    let registry = VIDEOS.with_borrow_mut(|slot| Arc::downgrade(slot.get_or_insert_with(Videos::default)));
    Ok(vm.later(move || {
        let video = rsui::video::decode(std::path::Path::new(&path), options)?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let text = serde_json::json!({
            "id": id, "width": video.width, "height": video.height, "fps": video.fps, "count": video.frames.len(),
        })
        .to_string();
        // An in-flight decode belongs to the app generation that requested it.
        if let Some(registry) = registry.upgrade() {
            registry.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id, video);
        }
        Ok(Some(text))
    }))
}

/// `videoRead(id, index, into)`: copies a frame's RGBA into a `Uint8Array` or `Uint8ClampedArray`, and says whether there was one.
fn host_read(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let index = number_at(vm, args, 1) as usize;
    let into = args.get(vm, 2);
    let copied = VIDEOS.with_borrow(|slot| {
        let Some(registry) = slot else {
            return false;
        };
        let videos = registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(frame) = videos.get(&id).and_then(|video| video.frames.get(index)) else {
            return false;
        };
        let Some(out) = vm.bytes_mut(into) else {
            return false;
        };
        let n = out.len().min(frame.len());
        out[..n].copy_from_slice(&frame[..n]);
        true
    });
    Ok(Value::bool(copied))
}

/// `videoFree(id)`: lets go of its frames.
fn host_free(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    VIDEOS.with_borrow(|slot| {
        if let Some(registry) = slot {
            registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
        }
    });
    Ok(Value::UNDEFINED)
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 3] = [("videoOpen", host_open), ("videoRead", host_read), ("videoFree", host_free)];
