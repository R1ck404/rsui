//! Playing sound, with what comes with the system. It all happens on a thread of its own, as opening a file can take a moment, and
//! what plays it needs a thread that's looking at its messages. There's no thread, and nothing's loaded, until something's played.

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP,
};
use windows::core::{HSTRING, s, w};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

/// A sound that's been opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundId(pub u32);

/// What to tell the thread.
enum Job {
    /// Do this, and tell this window when it's finished, if it's given.
    Say(String, isize),
    /// Open this, and note which device it is.
    Open(SoundId, String),
    /// Say what this comes to.
    Ask(String, Sender<Option<String>>),
    /// Forget about this.
    Close(SoundId),
}

struct Worker {
    jobs: Mutex<VecDeque<Job>>,
    thread: u32,
}

/// Which sound each device is playing, for when the system says that one has ended.
static DEVICES: Mutex<Option<HashMap<u32, SoundId>>> = Mutex::new(None);
static NEXT: AtomicU32 = AtomicU32::new(1);

type Send = unsafe extern "system" fn(*const u16, *mut u16, u32, HWND) -> u32;
type DeviceOf = unsafe extern "system" fn(*const u16) -> u32;

struct Functions {
    send: Send,
    device_of: DeviceOf,
}

fn functions() -> Option<&'static Functions> {
    static FOUND: OnceLock<Option<Functions>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            type Found = unsafe extern "system" fn() -> isize;
            // SAFETY: the library stays loaded for good, and these functions have these signatures.
            unsafe {
                let library = LoadLibraryW(w!("winmm.dll")).ok()?;
                Some(Functions {
                    send: std::mem::transmute::<Found, Send>(GetProcAddress(
                        library,
                        s!("mciSendStringW"),
                    )?),
                    device_of: std::mem::transmute::<Found, DeviceOf>(GetProcAddress(
                        library,
                        s!("mciGetDeviceIDW"),
                    )?),
                })
            }
        })
        .as_ref()
}

fn say(command: &str, tell: isize) -> Result<String, u32> {
    let functions = functions().ok_or(0u32)?;
    let wide = HSTRING::from(command);
    let mut answer = [0u16; 128];
    // SAFETY: both last through the call, the room for the answer is what's said, and the window's only posted to.
    let failed = unsafe {
        (functions.send)(
            wide.as_ptr(),
            answer.as_mut_ptr(),
            answer.len() as u32,
            HWND(tell as *mut c_void),
        )
    };
    if failed != 0 {
        return Err(failed);
    }
    let end = answer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(answer.len());
    Ok(String::from_utf16_lossy(&answer[..end]))
}

fn run_job(job: Job) {
    match job {
        Job::Say(command, tell) => {
            if let Err(error) = say(&command, tell) {
                eprintln!("A sound couldn't do this: {command} (error {error})");
            }
        }
        Job::Open(id, path) => {
            let name = alias(id);
            if let Err(error) = say(&open_command(&path, &name), 0) {
                eprintln!("{path} couldn't be opened to play it (error {error})");
                return;
            }
            let _ = say(&format!("set {name} time format milliseconds"), 0);
            if let Some(functions) = functions() {
                let wide = HSTRING::from(name.as_str());
                // SAFETY: this lasts through the call.
                let device = unsafe { (functions.device_of)(wide.as_ptr()) };
                let mut devices = DEVICES.lock().expect("lock is not poisoned");
                devices.get_or_insert_default().insert(device, id);
            }
        }
        Job::Ask(command, answer) => {
            let _ = answer.send(say(&command, 0).ok());
        }
        Job::Close(id) => {
            let _ = say(&format!("close {}", alias(id)), 0);
            let mut devices = DEVICES.lock().expect("lock is not poisoned");
            if let Some(devices) = devices.as_mut() {
                devices.retain(|_, sound| *sound != id);
            }
        }
    }
}

fn worker() -> Option<&'static Worker> {
    static STARTED: OnceLock<Option<Worker>> = OnceLock::new();
    STARTED
        .get_or_init(|| {
            let (ready, wait) = channel();
            std::thread::Builder::new()
                .name("rsui sound".into())
                .stack_size(1024 * 1024)
                .spawn(move || {
                    let mut message = MSG::default();
                    // SAFETY: `message` is valid to write to. Looking makes the thread's list of messages, which it needs before
                    // anything can be sent to it, and this goes on for as long as the program does.
                    unsafe {
                        let _ = PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE);
                        let _ = ready.send(GetCurrentThreadId());
                        while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
                            if message.hwnd.is_invalid() && message.message == WM_APP {
                                while let Some(job) = next_job() {
                                    run_job(job);
                                }
                            } else {
                                DispatchMessageW(&raw const message);
                            }
                        }
                    }
                })
                .ok()?;
            Some(Worker {
                jobs: Mutex::default(),
                thread: wait.recv().ok()?,
            })
        })
        .as_ref()
}

fn next_job() -> Option<Job> {
    worker()?
        .jobs
        .lock()
        .expect("lock is not poisoned")
        .pop_front()
}

fn send(job: Job) -> bool {
    let Some(worker) = worker() else {
        return false;
    };
    worker
        .jobs
        .lock()
        .expect("lock is not poisoned")
        .push_back(job);
    // SAFETY: this only posts a message, and does nothing if the thread's gone.
    unsafe { PostThreadMessageW(worker.thread, WM_APP, WPARAM(0), LPARAM(0)).is_ok() }
}

fn alias(id: SoundId) -> String {
    format!("rsui{}", id.0)
}

/// What opens a file. The kind's what lets this play MP3s as well as WAVs, and a path can't have a quote in it on Windows.
fn open_command(path: &str, name: &str) -> String {
    format!("open \"{path}\" type mpegvideo alias {name}")
}

fn volume_command(name: &str, volume: f32) -> String {
    format!(
        "setaudio {name} volume to {}",
        (volume.clamp(0.0, 1.0) * 1000.0).round() as u32
    )
}

/// Which sound the system means when it says this device has finished.
pub(crate) fn sound_of_device(device: u32) -> Option<SoundId> {
    DEVICES
        .lock()
        .expect("lock is not poisoned")
        .as_ref()?
        .get(&device)
        .copied()
}

/// Opens a file to play it, in the background. Anything that goes wrong is said on the standard error.
pub fn open_sound(path: &Path) -> SoundId {
    let id = SoundId(NEXT.fetch_add(1, Ordering::Relaxed));
    send(Job::Open(id, path.to_string_lossy().into_owned()));
    id
}

/// Plays it from the start, or from where it was paused. `tell` is the window that hears when it's finished.
pub(crate) fn play(id: SoundId, from_start: bool, tell: isize) {
    let name = alias(id);
    let command = if from_start {
        format!("play {name} from 0 notify")
    } else {
        format!("play {name} notify")
    };
    send(Job::Say(command, tell));
}

pub fn pause_sound(id: SoundId) {
    send(Job::Say(format!("pause {}", alias(id)), 0));
}

pub fn stop_sound(id: SoundId) {
    send(Job::Say(format!("stop {}", alias(id)), 0));
}

/// From 0 to 1.
pub fn set_sound_volume(id: SoundId, volume: f32) {
    send(Job::Say(volume_command(&alias(id), volume), 0));
}

/// Moves it to here, in milliseconds, for when it next plays.
pub fn seek_sound(id: SoundId, at: u64) {
    send(Job::Say(format!("seek {} to {at}", alias(id)), 0));
}

/// Lets go of it.
pub fn close_sound(id: SoundId) {
    send(Job::Close(id));
}

fn ask_number(command: String) -> Option<u64> {
    let (answer, wait) = channel();
    send(Job::Ask(command, answer));
    // Anything that's opening a file is going to be done long before this, and nothing here should hold a window up for long.
    wait.recv_timeout(Duration::from_millis(250))
        .ok()??
        .parse()
        .ok()
}

/// How long it is, in milliseconds, if that's known yet.
pub fn sound_length(id: SoundId) -> Option<u64> {
    ask_number(format!("status {} length", alias(id)))
}

/// How far it's got, in milliseconds.
pub fn sound_position(id: SoundId) -> Option<u64> {
    ask_number(format!("status {} position", alias(id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_said_to_the_system_is_written_as_it_wants() {
        assert_eq!(
            open_command("C:\\a folder\\bell.mp3", "rsui3"),
            "open \"C:\\a folder\\bell.mp3\" type mpegvideo alias rsui3"
        );
        assert_eq!(volume_command("rsui3", 0.5), "setaudio rsui3 volume to 500");
        assert_eq!(
            volume_command("rsui3", 2.0),
            "setaudio rsui3 volume to 1000"
        );
        assert_eq!(volume_command("rsui3", -1.0), "setaudio rsui3 volume to 0");
    }

    #[test]
    fn asking_about_a_sound_that_is_not_there_gives_nothing() {
        // Nothing's opened, so nothing can play.
        assert_eq!(sound_length(SoundId(999_999)), None);
        assert_eq!(sound_of_device(999_999), None);
    }

    /// A WAV file of nothing at all.
    fn silence(milliseconds: u32) -> Vec<u8> {
        const RATE: u32 = 8000;
        let samples = RATE * milliseconds / 1000;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + samples).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&samples.to_le_bytes());
        // In this format, the middle is 128.
        wav.resize(wav.len() + samples as usize, 128);
        wav
    }

    /// This uses the machine's sound device, though nobody could hear it, so it's only for running by hand.
    #[test]
    #[ignore = "it uses the sound device, though what it plays is silent"]
    fn a_silent_file_opens_says_how_long_it_is_and_plays_to_the_end() {
        use std::time::Instant;
        let file = std::env::temp_dir().join(format!("rsui-silence-{}.wav", std::process::id()));
        std::fs::write(&file, silence(300)).unwrap();
        let id = open_sound(&file);
        let length = sound_length(id).expect("it says how long it is");
        assert!((250..=350).contains(&length), "{length}");
        crate::desktop::tests_support::expect_sound_to_end(id);
        let started = Instant::now();
        close_sound(id);
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(100));
        let _ = std::fs::remove_file(file);
    }
}
