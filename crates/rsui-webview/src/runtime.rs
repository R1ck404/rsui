//! Finding Edge's WebView2 runtime, which comes with Windows 11 and is kept up to date with Edge, and starting it.
//!
//! WebView2's own loader, `WebView2Loader.dll`, is something an app ships beside itself, and only reads the registry and
//! calls into the runtime. So this does the same, as the loader that comes with the `webview` library does: the runtime's
//! folder is in the registry, under EdgeUpdate, and its `EmbeddedBrowserWebView.dll` makes environments. A loader beside the
//! program is used instead, if there is one, as that's what WebView2 documents. A runtime in a folder of its own, a "fixed
//! version" one, is used if `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` names it, as with WebView2's loader.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
    RegQueryValueExW,
};
use windows::core::{HRESULT, HSTRING, PCWSTR, s};

use crate::ffi::{ICoreWebView2Environment, handlers};
use crate::handler::{self, Made};

/// The stable runtime's id with EdgeUpdate. Beta, Dev and Canary have their own, which aren't used, as WebView2 only uses
/// those when told to.
const STABLE: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
/// The oldest runtime with everything this uses, which is version 10 of the environment: profiles.
const OLDEST: [u32; 4] = [101, 0, 1210, 39];

/// Where a runtime is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub version: String,
    /// The library that makes environments.
    pub client: PathBuf,
    /// Whether it's a runtime in a folder of its own, rather than the one that's installed.
    pub fixed: bool,
}

/// The parts of a version, as WebView2 says them: `157.0.4341.42`.
pub(crate) fn parse_version(text: &str) -> Option<[u32; 4]> {
    let mut parts = [0u32; 4];
    let mut count = 0;
    for part in text.trim().split('.') {
        if count == 4 {
            return None;
        }
        parts[count] = part.parse().ok()?;
        count += 1;
    }
    (count == 4).then_some(parts)
}

/// The library inside a runtime's folder that makes environments.
pub(crate) fn client_in(folder: &Path) -> PathBuf {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x64"
    };
    folder.join("EBWebView").join(arch).join("EmbeddedBrowserWebView.dll")
}

/// The runtime that would be used, if there's one new enough.
pub fn find() -> Option<Runtime> {
    if let Some(folder) = std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").filter(|folder| !folder.is_empty()) {
        let folder = PathBuf::from(folder);
        let version = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let client = client_in(&folder);
        return client.is_file().then_some(Runtime { version, client, fixed: true });
    }
    // Installed for everyone, or for this person only.
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let key = format!("SOFTWARE\\Microsoft\\EdgeUpdate\\ClientState\\{STABLE}");
        let Some(folder) = registry_text(root, &key, "EBWebView") else {
            continue;
        };
        let folder = PathBuf::from(folder);
        let version = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        if parse_version(&version).is_none_or(|parts| parts < OLDEST) {
            continue;
        }
        let client = client_in(&folder);
        if client.is_file() {
            return Some(Runtime { version, client, fixed: false });
        }
    }
    None
}

fn registry_text(root: HKEY, key: &str, value: &str) -> Option<String> {
    let mut opened = HKEY::default();
    // SAFETY: the key is closed before this returns, and what's read fits the buffer given.
    unsafe {
        if RegOpenKeyExW(root, &HSTRING::from(key), None, KEY_READ | KEY_WOW64_32KEY, &mut opened) != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u16; 1024];
        let mut bytes = (buffer.len() * 2) as u32;
        let mut kind = REG_VALUE_TYPE::default();
        let read = RegQueryValueExW(
            opened,
            &HSTRING::from(value),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        );
        let _ = RegCloseKey(opened);
        if read != ERROR_SUCCESS {
            return None;
        }
        let length = (bytes as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..length]);
        let text = text.trim_end_matches('\0').to_owned();
        (!text.is_empty()).then_some(text)
    }
}

/// WebView2's own loader's `CreateCoreWebView2EnvironmentWithOptions`: the runtime's folder, the data folder, options and the
/// handler.
type LoaderCreate = unsafe extern "system" fn(PCWSTR, PCWSTR, *mut c_void, *mut c_void) -> HRESULT;
/// The runtime's own, as the loader calls it: whether to look for a running browser process with the same folder, whether the
/// runtime's installed (0) or in a folder of its own (1), the data folder, options and the handler.
type RuntimeCreate = unsafe extern "system" fn(i32, i32, PCWSTR, *mut c_void, *mut c_void) -> HRESULT;
/// What a library gives for a function.
type Found = unsafe extern "system" fn() -> isize;
/// What's told an environment, once there is one or there can't be.
type Then = Box<dyn FnOnce(Result<ICoreWebView2Environment, String>)>;

/// How an environment's made.
enum Maker {
    Loader(LoaderCreate),
    Runtime(RuntimeCreate, i32),
}

/// What there is to make environments with, found the first time one's wanted, and kept for good.
fn maker() -> Result<&'static Maker, String> {
    thread_local! {
        static MAKER: RefCell<Option<Result<&'static Maker, String>>> = const { RefCell::new(None) };
    }
    MAKER.with(|maker| maker.borrow_mut().get_or_insert_with(load_maker).clone())
}

fn library(path: &Path) -> Option<HMODULE> {
    // SAFETY: loading a library, which stays loaded for good.
    unsafe { LoadLibraryW(&HSTRING::from(path.as_os_str())).ok() }
}

fn load_maker() -> Result<&'static Maker, String> {
    let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("WebView2Loader.dll")));
    if let Some(module) = beside.filter(|loader| loader.is_file()).and_then(|loader| library(&loader)) {
        // SAFETY: the loader has this function with this signature, and stays loaded.
        if let Some(found) = unsafe { GetProcAddress(module, s!("CreateCoreWebView2EnvironmentWithOptions")) } {
            // SAFETY: as above.
            let made = unsafe { std::mem::transmute::<Found, LoaderCreate>(found) };
            crate::log(|| "using WebView2Loader.dll beside the program".to_owned());
            return Ok(Box::leak(Box::new(Maker::Loader(made))));
        }
    }
    let runtime = find().ok_or_else(missing)?;
    crate::log(|| format!("using the runtime {} at {}", runtime.version, runtime.client.display()));
    let module = library(&runtime.client).ok_or_else(|| format!("{} couldn't be loaded", runtime.client.display()))?;
    // SAFETY: the runtime has this function with this signature, as the loader expects, and stays loaded.
    let found = unsafe { GetProcAddress(module, s!("CreateWebViewEnvironmentWithOptionsInternal")) }
        .ok_or_else(|| format!("{} isn't a WebView2 runtime this can use", runtime.client.display()))?;
    // SAFETY: as above.
    let made = unsafe { std::mem::transmute::<Found, RuntimeCreate>(found) };
    Ok(Box::leak(Box::new(Maker::Runtime(made, i32::from(runtime.fixed)))))
}

/// What's said when there's no runtime.
pub fn missing() -> String {
    "This needs Microsoft Edge WebView2, which comes with Windows 11 and isn't on this computer. It can be installed from \
     https://go.microsoft.com/fwlink/p/?LinkId=2124703"
        .to_owned()
}

/// An environment: the browser processes for one folder of data, which every view of it shares.
enum Environment {
    /// What's waiting for it, and the handler WebView2 calls when it's there, which lives as long, once it's been made.
    Starting(Vec<Then>, #[allow(dead_code)] Option<Made>),
    Ready(ICoreWebView2Environment),
    Failed(String),
}

thread_local! {
    static ENVIRONMENTS: RefCell<Vec<(PathBuf, Environment)>> = const { RefCell::new(Vec::new()) };
}

/// Calls `then` with the environment for `folder` once it's there, starting it if it isn't. It's on this thread, which
/// must be one with a message loop, as WebView2's are.
pub(crate) fn environment(folder: &Path, then: impl FnOnce(Result<ICoreWebView2Environment, String>) + 'static) {
    environment_with(folder, then, start);
}

/// [`environment`], starting one with `start`, which may answer, with [`settle`], before it returns, as WebView2 does.
fn environment_with(
    folder: &Path,
    then: impl FnOnce(Result<ICoreWebView2Environment, String>) + 'static,
    start: impl FnOnce(&Path) -> Result<(), String>,
) {
    enum Now {
        Have(Result<ICoreWebView2Environment, String>),
        Waiting,
        Start,
    }
    let mut then: Option<Then> = Some(Box::new(then));
    let now = ENVIRONMENTS.with(|all| {
        let mut all = all.borrow_mut();
        match all.iter_mut().find(|(at, _)| at == folder) {
            Some((_, Environment::Ready(environment))) => Now::Have(Ok(environment.clone())),
            Some((_, Environment::Failed(error))) => Now::Have(Err(error.clone())),
            Some((_, Environment::Starting(waiting, _))) => {
                waiting.extend(then.take());
                Now::Waiting
            }
            None => Now::Start,
        }
    });
    let Some(then) = then else {
        return;
    };
    match now {
        Now::Have(result) => return then(result),
        Now::Waiting => return,
        Now::Start => {}
    }
    // What's waiting is there before WebView2 is asked, as it can answer before it returns.
    ENVIRONMENTS.with(|all| all.borrow_mut().push((folder.to_owned(), Environment::Starting(vec![then], None))));
    if let Err(error) = start(folder) {
        crate::log(|| format!("the environment for {} couldn't be started: {error}", folder.display()));
        settle(folder, Err(error));
    }
}

/// Tells everything waiting for the environment for `folder` what came of starting it, and keeps that.
fn settle(folder: &Path, result: Result<ICoreWebView2Environment, String>) {
    let waiting = ENVIRONMENTS.with(|all| {
        let mut all = all.borrow_mut();
        let entry = all.iter_mut().find(|(at, _)| at == folder)?;
        let now = match &result {
            Ok(environment) => Environment::Ready(environment.clone()),
            Err(error) => Environment::Failed(error.clone()),
        };
        match std::mem::replace(&mut entry.1, now) {
            Environment::Starting(waiting, _) => Some(waiting),
            _ => None,
        }
    });
    for then in waiting.unwrap_or_default() {
        then(result.clone());
    }
}

fn start(folder: &Path) -> Result<(), String> {
    let maker = maker()?;
    // WebView2 needs this thread to be a single-threaded apartment. It's fine if it already is; one that's multithreaded
    // can't have it.
    // SAFETY: initializing COM on this thread, which is never undone, as the thread's windows need it for as long as it runs.
    let started = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    crate::log(|| format!("COM on this thread as a single-threaded apartment: {started:?}"));
    if started.is_err() {
        return Err(format!("this thread can't have WebView2 in it: {}", started.message()));
    }
    let _ = std::fs::create_dir_all(folder);
    let key = folder.to_owned();
    let made = handler::make(handlers::ENVIRONMENT_CREATED, move |first, second| {
        let code = handler::code(first);
        crate::log(|| format!("the environment's made: {code:?}, {}", if second == 0 { "with nothing" } else { "with an environment" }));
        // SAFETY: WebView2 gives an environment, or nothing, with the result.
        let result = match unsafe { handler::borrowed::<ICoreWebView2Environment>(second) } {
            Some(environment) if code.is_ok() => Ok(environment),
            _ => Err(format!("WebView2 couldn't start ({:#010x}): {}", code.0 as u32, code.message())),
        };
        settle(&key, result);
        HRESULT(0)
    });
    let folder_name = HSTRING::from(folder.as_os_str());
    // SAFETY: the folder and handler live for the call, which holds on to the handler for itself.
    let result = unsafe {
        match maker {
            Maker::Loader(create) => create(PCWSTR::null(), PCWSTR(folder_name.as_ptr()), std::ptr::null_mut(), made.raw()),
            Maker::Runtime(create, kind) => create(1, *kind, PCWSTR(folder_name.as_ptr()), std::ptr::null_mut(), made.raw()),
        }
    };
    crate::log(|| format!("asked for an environment in {}: {result:?}", folder.display()));
    if result.is_err() {
        return Err(format!("WebView2 couldn't start ({:#010x}): {}", result.0 as u32, result.message()));
    }
    // The handler lives as long as it's waited for, unless it's been answered already.
    ENVIRONMENTS.with(|all| {
        if let Some((_, Environment::Starting(_, kept))) = all.borrow_mut().iter_mut().find(|(at, _)| at == folder) {
            *kept = Some(made);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_as_webview2_says_them() {
        assert_eq!(parse_version("157.0.4341.42"), Some([157, 0, 4341, 42]));
        assert_eq!(parse_version("157.0.4341"), None);
        assert_eq!(parse_version("157.0.4341.42.1"), None);
        assert_eq!(parse_version("x.0.0.0"), None);
        assert!(parse_version("100.0.1185.36").unwrap() < OLDEST);
        assert!(parse_version("157.0.4341.42").unwrap() >= OLDEST);
    }

    #[test]
    fn a_runtime_that_is_found_is_new_enough_and_there() {
        // Whether there's one depends on the computer; one that's found is one that can be used.
        if let Some(runtime) = find() {
            assert!(runtime.client.is_file(), "{runtime:?}");
            assert!(runtime.fixed || parse_version(&runtime.version).is_some_and(|parts| parts >= OLDEST), "{runtime:?}");
        }
    }

    #[test]
    fn an_environment_that_is_there_before_asking_for_it_returns_is_still_heard() {
        // WebView2 157 says it's made the environment from inside the call that asks for it.
        let folder = Path::new(r"C:\rsui-webview-test\answers-at-once");
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let told = heard.clone();
        environment_with(folder, move |result| told.borrow_mut().push(result.err()), |folder| {
            settle(folder, Err("made".to_owned()));
            Ok(())
        });
        assert_eq!(*heard.borrow(), vec![Some("made".to_owned())]);
        // And it's kept for what asks later.
        let told = heard.clone();
        environment_with(folder, move |result| told.borrow_mut().push(result.err()), |_| panic!("it's started once"));
        assert_eq!(heard.borrow().len(), 2);
    }

    #[test]
    fn the_library_is_where_the_runtime_keeps_it() {
        let client = client_in(Path::new(r"C:\Runtime\157.0.4341.42"));
        assert!(client.ends_with(r"EBWebView\x64\EmbeddedBrowserWebView.dll") || cfg!(not(target_arch = "x86_64")));
    }
}
