//! What programs ask of the system now and then. All of it loads what it needs only once it's asked
//! for, so that a program that never does doesn't load more libraries to start up.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{HSTRING, s, w};

// Kernelbase has this too, and it's always loaded, where advapi32 would load three more libraries just to start up.
#[link(name = "api-ms-win-core-registry-l1-1-0", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegGetValueW(
        key: isize,
        sub_key: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
}

/// Whether the system's set to dark colors for apps.
pub fn system_is_dark() -> bool {
    const CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    const ONLY_DWORDS: u32 = 0x10;
    let mut light = 1u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the names end in a terminator, and `light` is as big as `size` says.
    let result = unsafe {
        RegGetValueW(
            CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            w!("AppsUseLightTheme").as_ptr(),
            ONLY_DWORDS,
            std::ptr::null_mut(),
            (&raw mut light).cast(),
            &raw mut size,
        )
    };
    result == 0 && light == 0
}

/// Whether this is something it's safe to hand to whatever opens it. Text in a program can come from
/// anywhere, and what opens `file:` or a program's own scheme can run anything.
pub fn can_open_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    !rest.is_empty()
        && (scheme.eq_ignore_ascii_case("https")
            || scheme.eq_ignore_ascii_case("http")
            || scheme.eq_ignore_ascii_case("mailto"))
}

/// Opens a web page in the browser, or an address in the mail program, without waiting for that.
/// Anything else is ignored. See [`can_open_url`].
pub fn open_url(url: &str) {
    if !can_open_url(url) {
        return;
    }
    let url = HSTRING::from(url);
    // Starting a browser can take a while, and a window shouldn't stop for it.
    std::thread::spawn(move || {
        type ShellExecute = unsafe extern "system" fn(
            HWND,
            *const u16,
            *const u16,
            *const u16,
            *const u16,
            i32,
        ) -> isize;
        const SW_SHOWNORMAL: i32 = 1;
        // SAFETY: the library stays loaded for good, the function has this signature, and all that it's
        // given lasts through the call, with a null window and nulls for what it can do without.
        unsafe {
            let Ok(library) = LoadLibraryW(w!("shell32.dll")) else {
                return;
            };
            let Some(found) = GetProcAddress(library, s!("ShellExecuteW")) else {
                return;
            };
            let execute =
                std::mem::transmute::<unsafe extern "system" fn() -> isize, ShellExecute>(found);
            execute(
                HWND::default(),
                w!("open").as_ptr(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    });
}

type SetAttribute = unsafe extern "system" fn(HWND, u32, *const c_void, u32) -> i32;

fn set_attribute() -> Option<SetAttribute> {
    static FOUND: OnceLock<Option<SetAttribute>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        // SAFETY: the library stays loaded for good, and the function has this signature.
        unsafe {
            let library = LoadLibraryW(w!("dwmapi.dll")).ok()?;
            let found = GetProcAddress(library, s!("DwmSetWindowAttribute"))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                SetAttribute,
            >(found))
        }
    })
}

/// Gives a window with no frame what its frame would've come with: a shadow, and round corners on Windows 11. The system only draws a
/// shadow for a window that has some frame, so this says one pixel of it goes under what's inside, where it's drawn over. One covering
/// the whole screen has neither.
///
/// A window that's seen through, as one with a backdrop is, has its frame go all the way in, as the system only draws a backdrop
/// under the frame, and then shows it wherever what's drawn is see-through.
pub(crate) fn set_frameless_look(window: HWND, frameless: bool, fullscreen: bool, see_through: bool) {
    #[repr(C)]
    struct Margins {
        left: i32,
        right: i32,
        top: i32,
        bottom: i32,
    }
    type ExtendFrame = unsafe extern "system" fn(HWND, *const Margins) -> i32;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const CORNERS_AS_USUAL: i32 = 0;
    const SQUARE_CORNERS: i32 = 1;
    const ROUND_CORNERS: i32 = 2;
    static FOUND: OnceLock<Option<ExtendFrame>> = OnceLock::new();
    let extend = *FOUND.get_or_init(|| {
        // SAFETY: the library stays loaded for good, and the function has this signature.
        unsafe {
            let library = LoadLibraryW(w!("dwmapi.dll")).ok()?;
            let found = GetProcAddress(library, s!("DwmExtendFrameIntoClientArea"))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                ExtendFrame,
            >(found))
        }
    });
    let edge = if see_through { -1 } else { i32::from(frameless && !fullscreen) };
    let corners = match (frameless, fullscreen) {
        (false, _) => CORNERS_AS_USUAL,
        (true, false) => ROUND_CORNERS,
        (true, true) => SQUARE_CORNERS,
    };
    // SAFETY: what's given is the size that's said and lasts through each call, and a closed window, or a Windows 10 that doesn't have
    // the corners, only gives an error.
    unsafe {
        if let Some(extend) = extend {
            let margins = Margins {
                left: edge,
                right: edge,
                top: edge,
                bottom: edge,
            };
            extend(window, &raw const margins);
        }
        if let Some(set) = set_attribute() {
            set(
                window,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corners).cast(),
                size_of::<i32>() as u32,
            );
        }
    }
}

/// Has the system draw nothing of its own around a window, or what it usually does: no shadow, no edge and no round corners,
/// for a transparent window, whose rectangle would otherwise show around what's drawn in it.
pub(crate) fn set_bare_look(window: HWND, bare: bool) {
    const DWMWA_NCRENDERING_POLICY: u32 = 2;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    let Some(set) = set_attribute() else {
        return;
    };
    // As the window's style says, or not at all; square corners, or as usual; no edge, or the system's.
    let (policy, corners, border) = if bare { (1i32, 1i32, 0xFFFF_FFFEu32) } else { (0, 0, 0xFFFF_FFFF) };
    // SAFETY: each value is the size that's said, and a closed window, or a Windows 10 that doesn't have some, only gives errors.
    unsafe {
        set(window, DWMWA_NCRENDERING_POLICY, (&raw const policy).cast(), size_of::<i32>() as u32);
        set(window, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const corners).cast(), size_of::<i32>() as u32);
        set(window, DWMWA_BORDER_COLOR, (&raw const border).cast(), size_of::<u32>() as u32);
    }
}

/// Makes a window's title bar dark or light, to go with what's in it.
pub(crate) fn set_dark_title_bar(window: HWND, dark: bool) {
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    if let Some(set) = set_attribute() {
        let value = i32::from(dark);
        // SAFETY: `value` is the size that's said, and a closed window only gives an error.
        unsafe {
            set(
                window,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const value).cast(),
                size_of::<i32>() as u32,
            );
        }
    }
}

/// Has the system draw this behind a window, through what isn't drawn over it: `mica`, `acrylic`, `tabbed` or `none`, with
/// the title bar dark or light as the system's colors are, and says whether the system took it. It's only on Windows 11, from
/// 22H2, and anything else gives nothing.
pub(crate) fn set_backdrop(window: HWND, kind: &str) -> bool {
    use windows::Win32::Graphics::Dwm::{
        DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TABBEDWINDOW, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    };
    let backdrop = match kind {
        "mica" => DWMSBT_MAINWINDOW,
        "acrylic" => DWMSBT_TRANSIENTWINDOW,
        "tabbed" => DWMSBT_TABBEDWINDOW,
        _ => DWMSBT_NONE,
    };
    set_dark_title_bar(window, system_is_dark());
    let Some(set) = set_attribute() else {
        return false;
    };
    // SAFETY: the value is the size that's said, and a closed window, or a Windows that doesn't have this, only gives an error.
    unsafe {
        set(
            window,
            DWMWA_SYSTEMBACKDROP_TYPE.0 as u32,
            (&raw const backdrop.0).cast(),
            size_of::<i32>() as u32,
        ) >= 0
    }
}

/// Says where the caret is, for what shows what's being composed in Chinese, Japanese or Korean, which goes there rather
/// than in a corner of the window. This has what it needs loaded only once someone's composing something.
pub(crate) fn place_composition(window: HWND, x: f32, y: f32, height: f32) -> bool {
    #[repr(C)]
    struct CompositionForm {
        style: u32,
        at: POINT,
        area: [i32; 4],
    }
    #[repr(C)]
    struct CandidateForm {
        index: u32,
        style: u32,
        at: POINT,
        area: [i32; 4],
    }
    struct Functions {
        get_context: unsafe extern "system" fn(HWND) -> *mut c_void,
        release_context: unsafe extern "system" fn(HWND, *mut c_void) -> i32,
        set_composition: unsafe extern "system" fn(*mut c_void, *const CompositionForm) -> i32,
        set_candidate: unsafe extern "system" fn(*mut c_void, *const CandidateForm) -> i32,
    }
    static FOUND: OnceLock<Option<Functions>> = OnceLock::new();
    type Found = unsafe extern "system" fn() -> isize;
    let Some(functions) = FOUND.get_or_init(|| {
        // SAFETY: the library stays loaded for good, and these functions have these signatures.
        unsafe {
            let library = LoadLibraryW(w!("imm32.dll")).ok()?;
            Some(Functions {
                get_context: std::mem::transmute::<
                    Found,
                    unsafe extern "system" fn(HWND) -> *mut c_void,
                >(GetProcAddress(library, s!("ImmGetContext"))?),
                release_context: std::mem::transmute::<
                    Found,
                    unsafe extern "system" fn(HWND, *mut c_void) -> i32,
                >(GetProcAddress(
                    library,
                    s!("ImmReleaseContext"),
                )?),
                set_composition: std::mem::transmute::<
                    Found,
                    unsafe extern "system" fn(*mut c_void, *const CompositionForm) -> i32,
                >(GetProcAddress(
                    library,
                    s!("ImmSetCompositionWindow"),
                )?),
                set_candidate: std::mem::transmute::<
                    Found,
                    unsafe extern "system" fn(*mut c_void, *const CandidateForm) -> i32,
                >(GetProcAddress(
                    library,
                    s!("ImmSetCandidateWindow"),
                )?),
            })
        }
    }) else {
        return false;
    };
    const AT_A_POINT: u32 = 2;
    const AWAY_FROM_AREA: u32 = 0x80;
    let (left, top, bottom) = (x as i32, y as i32, (y + height) as i32);
    let composition = CompositionForm {
        style: AT_A_POINT,
        at: POINT { x: left, y: top },
        area: [0; 4],
    };
    // The list of what could be meant goes below the line, and keeps off the line itself.
    let candidates = CandidateForm {
        index: 0,
        style: AWAY_FROM_AREA,
        at: POINT { x: left, y: bottom },
        area: [left, top, left + 1, bottom],
    };
    // SAFETY: what's given lasts through each call, and the context is given back. A window with no context
    // has nothing to set.
    unsafe {
        let context = (functions.get_context)(window);
        if context.is_null() {
            return false;
        }
        let placed = (functions.set_composition)(context, &raw const composition) != 0;
        (functions.set_candidate)(context, &raw const candidates);
        (functions.release_context)(window, context);
        placed
    }
}

/// What's being put together by a way of typing, as Chinese, Japanese and Korean are, so far, or what it came to: `which`
/// is the system's `GCS_COMPSTR` or `GCS_RESULTSTR`. This has what it needs loaded only once someone's composing something.
pub(crate) fn composition_text(window: HWND, which: u32) -> Option<String> {
    struct Functions {
        get_context: unsafe extern "system" fn(HWND) -> *mut c_void,
        release_context: unsafe extern "system" fn(HWND, *mut c_void) -> i32,
        get_string: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
    }
    static FOUND: OnceLock<Option<Functions>> = OnceLock::new();
    type Found = unsafe extern "system" fn() -> isize;
    let functions = FOUND
        .get_or_init(|| {
            // SAFETY: the library stays loaded for good, and these functions have these signatures.
            unsafe {
                let library = LoadLibraryW(w!("imm32.dll")).ok()?;
                Some(Functions {
                    get_context: std::mem::transmute::<
                        Found,
                        unsafe extern "system" fn(HWND) -> *mut c_void,
                    >(GetProcAddress(library, s!("ImmGetContext"))?),
                    release_context: std::mem::transmute::<
                        Found,
                        unsafe extern "system" fn(HWND, *mut c_void) -> i32,
                    >(GetProcAddress(library, s!("ImmReleaseContext"))?),
                    get_string: std::mem::transmute::<
                        Found,
                        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
                    >(GetProcAddress(library, s!("ImmGetCompositionStringW"))?),
                })
            }
        })
        .as_ref()?;
    // SAFETY: the first call says how many bytes there are, and the second writes no more than that into a buffer that
    // big. The context is given back.
    unsafe {
        let context = (functions.get_context)(window);
        if context.is_null() {
            return None;
        }
        let bytes = (functions.get_string)(context, which, std::ptr::null_mut(), 0);
        let mut units = vec![0u16; (bytes.max(0) as usize).div_ceil(2)];
        if bytes > 0 {
            (functions.get_string)(context, which, units.as_mut_ptr().cast(), bytes as u32);
        }
        (functions.release_context)(window, context);
        (bytes >= 0).then(|| String::from_utf16_lossy(&units))
    }
}

struct DropFunctions {
    query_file: unsafe extern "system" fn(*mut c_void, u32, *mut u16, u32) -> u32,
    query_point: unsafe extern "system" fn(*mut c_void, *mut POINT) -> i32,
    finish: unsafe extern "system" fn(*mut c_void),
}

fn drop_functions() -> Option<&'static DropFunctions> {
    static FOUND: OnceLock<Option<DropFunctions>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            // SAFETY: the library stays loaded for good, and these functions have these signatures.
            unsafe {
                let library = LoadLibraryW(w!("shell32.dll")).ok()?;
                let query_file = GetProcAddress(library, s!("DragQueryFileW"))?;
                let query_point = GetProcAddress(library, s!("DragQueryPoint"))?;
                let finish = GetProcAddress(library, s!("DragFinish"))?;
                type Found = unsafe extern "system" fn() -> isize;
                Some(DropFunctions {
                    query_file: std::mem::transmute::<
                        Found,
                        unsafe extern "system" fn(*mut c_void, u32, *mut u16, u32) -> u32,
                    >(query_file),
                    query_point: std::mem::transmute::<
                        Found,
                        unsafe extern "system" fn(*mut c_void, *mut POINT) -> i32,
                    >(query_point),
                    finish: std::mem::transmute::<Found, unsafe extern "system" fn(*mut c_void)>(
                        finish,
                    ),
                })
            }
        })
        .as_ref()
}

/// What was dropped on a window, and where, in physical pixels. The handle's used up by this.
///
/// # Safety
///
/// `dropped` must be what came with `WM_DROPFILES`, which hasn't been used up already.
pub(crate) unsafe fn take_dropped_files(dropped: *mut c_void) -> (Vec<String>, f32, f32) {
    let Some(functions) = drop_functions() else {
        return (Vec::new(), 0.0, 0.0);
    };
    let mut point = POINT::default();
    // SAFETY: the handle's good, by what this asks for, and it's finished with once its names are read.
    unsafe {
        (functions.query_point)(dropped, &raw mut point);
        let paths = dropped_file_names(dropped);
        (functions.finish)(dropped);
        (paths, point.x as f32, point.y as f32)
    }
}

/// The names of the files in what's dropped, from a handle to it, which is still there after.
///
/// # Safety
///
/// `dropped` is a handle to what the system says is dropped, which is there for as long as this runs.
pub(crate) unsafe fn dropped_file_names(dropped: *mut c_void) -> Vec<String> {
    let Some(functions) = drop_functions() else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    // SAFETY: the handle's good, by what this asks for, and each call has room for what it says.
    unsafe {
        let count = (functions.query_file)(dropped, u32::MAX, std::ptr::null_mut(), 0);
        for index in 0..count {
            let length = (functions.query_file)(dropped, index, std::ptr::null_mut(), 0);
            let mut path = vec![0u16; length as usize + 1];
            (functions.query_file)(dropped, index, path.as_mut_ptr(), length + 1);
            path.truncate(length as usize);
            paths.push(String::from_utf16_lossy(&path));
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

    #[test]
    fn only_the_web_and_mail_can_be_opened() {
        assert!(can_open_url("https://example.com"));
        assert!(can_open_url("HTTP://example.com"));
        assert!(can_open_url("mailto:someone@example.com"));
        assert!(!can_open_url("file:///C:/Windows/System32/calc.exe"));
        assert!(!can_open_url("C:\\Windows\\System32\\calc.exe"));
        assert!(!can_open_url("calc.exe"));
        assert!(!can_open_url("ms-settings:"));
        assert!(!can_open_url("javascript:alert(1)"));
        assert!(!can_open_url("https:"));
        assert!(!can_open_url(""));
    }

    #[test]
    fn asking_whether_the_system_is_dark_gives_an_answer() {
        let first = system_is_dark();
        assert_eq!(system_is_dark(), first);
    }

    #[test]
    fn what_composes_text_is_told_where_the_caret_is() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
        };
        #[repr(C)]
        #[derive(Default)]
        struct Form {
            style: u32,
            at: POINT,
            area: [i32; 4],
        }
        #[link(name = "imm32")]
        unsafe extern "system" {
            fn ImmGetContext(window: HWND) -> *mut c_void;
            fn ImmGetCompositionWindow(context: *mut c_void, form: *mut Form) -> i32;
            fn ImmReleaseContext(window: HWND, context: *mut c_void) -> i32;
        }
        // SAFETY: this window is never shown, and it's only used on this thread until it's destroyed. What's read
        // is the size that call writes.
        unsafe {
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let context = ImmGetContext(window);
            // Some setups have nothing for composing text at all, and then there's nothing to tell.
            if !context.is_null() {
                assert!(place_composition(window, 30.0, 40.0, 16.0));
                let mut form = Form::default();
                assert_ne!(ImmGetCompositionWindow(context, &raw mut form), 0);
                assert_eq!((form.at.x, form.at.y), (30, 40));
                ImmReleaseContext(window, context);
            }
            let _ = DestroyWindow(window);
        }
    }

    /// `DROPFILES` from the system's headers: where the names start, a point, whether that's on the
    /// edge of the window, and whether they're wide. Then names, each ending in a terminator, and one
    /// more after the last.
    fn made_up_drop(paths: &[&str], x: i32, y: i32) -> *mut c_void {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&20u32.to_le_bytes());
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for path in paths {
            for unit in path.encode_utf16().chain([0]) {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
        // SAFETY: this is the size of what's copied into it, and it stays allocated for what's using it.
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).unwrap();
            let locked = GlobalLock(memory).cast::<u8>();
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), locked, bytes.len());
            let _ = GlobalUnlock(memory);
            memory.0
        }
    }

    #[test]
    fn dropped_files_come_out_with_where_they_were_dropped() {
        let dropped = made_up_drop(&["C:\\a folder\\one.txt", "C:\\\u{4F60}\\two.md"], 30, 40);
        // SAFETY: it's in the form the system would make, and hasn't been used.
        let (paths, x, y) = unsafe { take_dropped_files(dropped) };
        assert_eq!(paths, ["C:\\a folder\\one.txt", "C:\\\u{4F60}\\two.md"]);
        assert_eq!((x, y), (30.0, 40.0));
    }
}
