//! An rsui app on Android. Android starts the app's activity, rsui's own (`java/dev/rsui/RsuiActivity.java`), which loads
//! this library and tells it everything that happens to it through the `native…` functions here. When the activity's made,
//! this copies the app's files out of the APK into the app's own folder, the first time, and runs the app there with
//! `rsui-tsx`, on a thread of its own, in the activity's window, until the activity ends.
//!
//! What's called here comes through JNI, whose functions are found by their place in the table Java hands over; the few
//! that are used are named below, with their places as `jni.h` has them.
//!
//! `tools/android/build-apk.ps1` makes the APK: the activity, this library for arm64, the app's files as assets with a list
//! of them in `rsui-assets.txt`, and a manifest.

#[cfg(target_os = "android")]
mod app {
    #![allow(unsafe_code, non_snake_case)]

    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;

    #[global_allocator]
    static ALLOCATOR: rsui::UiAlloc = rsui::UiAlloc;

    type Env = *mut *const *const c_void;
    type Object = *mut c_void;

    #[link(name = "android")]
    unsafe extern "C" {
        fn ANativeWindow_fromSurface(env: Env, surface: Object) -> *mut c_void;
        fn AAssetManager_fromJava(env: Env, manager: Object) -> *mut c_void;
    }

    /// The JNI function at `place` in the environment's table, as the type it is.
    ///
    /// # Safety
    /// `env` is a live environment, and `F` is the function's type at that place.
    unsafe fn jni<F: Copy>(env: Env, place: usize) -> F {
        // SAFETY: as the caller promises; the table's entries are all function pointers.
        unsafe { std::mem::transmute_copy(&*(*env).add(place)) }
    }

    const NEW_GLOBAL_REF: usize = 21;
    const DELETE_LOCAL_REF: usize = 23;
    const EXCEPTION_CLEAR: usize = 17;
    const GET_OBJECT_CLASS: usize = 31;
    const GET_METHOD_ID: usize = 33;
    const CALL_VOID_METHOD_A: usize = 63;
    const NEW_STRING: usize = 163;
    const GET_STRING_LENGTH: usize = 164;
    const GET_STRING_CHARS: usize = 165;
    const RELEASE_STRING_CHARS: usize = 166;
    const GET_JAVA_VM: usize = 219;
    const EXCEPTION_CHECK: usize = 228;
    // The VM's own table.
    const ATTACH_CURRENT_THREAD: usize = 4;
    const DETACH_CURRENT_THREAD: usize = 5;
    const GET_ENV: usize = 6;
    const JNI_VERSION_1_6: i32 = 0x0001_0006;

    /// The activity, kept for calls back into it from the app's thread.
    struct Activity {
        vm: usize,
        activity: usize,
        show_keyboard: usize,
        finish: usize,
        popup: usize,
        toast: usize,
        bar_colors: usize,
    }

    static ACTIVITY: OnceLock<Activity> = OnceLock::new();

    /// A Java string as Rust's.
    unsafe fn string(env: Env, text: Object) -> String {
        if text.is_null() {
            return String::new();
        }
        // SAFETY: the string is live for this call; its characters are let go of once they're copied.
        unsafe {
            let length = jni::<unsafe extern "C" fn(Env, Object) -> i32>(env, GET_STRING_LENGTH)(env, text);
            let units = jni::<unsafe extern "C" fn(Env, Object, *mut u8) -> *const u16>(env, GET_STRING_CHARS)(env, text, std::ptr::null_mut());
            if units.is_null() {
                return String::new();
            }
            let copied = String::from_utf16_lossy(std::slice::from_raw_parts(units, length.max(0) as usize));
            jni::<unsafe extern "C" fn(Env, Object, *const u16)>(env, RELEASE_STRING_CHARS)(env, text, units);
            copied
        }
    }

    /// Calls one of the activity's methods that take a boolean, or nothing, from whatever thread this is.
    fn call_activity(method: fn(&Activity) -> usize, argument: Option<bool>) {
        let Some(found) = ACTIVITY.get() else { return };
        let vm = found.vm as *mut *const *const c_void;
        let mut env: Env = std::ptr::null_mut();
        // SAFETY: the VM is the process's, which lasts; a thread that's attached already is given its environment again.
        unsafe {
            let attach: unsafe extern "C" fn(*mut *const *const c_void, *mut Env, *mut c_void) -> i32 =
                std::mem::transmute_copy(&*(*vm).add(ATTACH_CURRENT_THREAD));
            if attach(vm, &raw mut env, std::ptr::null_mut()) != 0 || env.is_null() {
                return;
            }
            let arguments = [argument.map_or(0u64, u64::from)];
            jni::<unsafe extern "C" fn(Env, Object, usize, *const u64)>(env, CALL_VOID_METHOD_A)(
                env,
                found.activity as Object,
                method(found),
                arguments.as_ptr(),
            );
            if jni::<unsafe extern "C" fn(Env) -> u8>(env, EXCEPTION_CHECK)(env) != 0 {
                jni::<unsafe extern "C" fn(Env)>(env, EXCEPTION_CLEAR)(env);
            }
        }
    }

    fn show_keyboard(show: bool) {
        call_activity(|found| found.show_keyboard, Some(show));
    }

    fn finish() {
        call_activity(|found| found.finish, None);
    }

    // The system's popups, which the activity shows on Android's main thread, and answers through `nativePopupAnswered`.

    /// Runs `call` with this thread's environment, attaching it to the VM for as long as that takes if it isn't already.
    fn with_env(call: impl FnOnce(Env, &Activity)) {
        let Some(found) = ACTIVITY.get() else { return };
        let vm = found.vm as *mut *const *const c_void;
        let mut env: Env = std::ptr::null_mut();
        // SAFETY: the VM is the process's, which lasts; a thread attached here is detached before this returns, and
        // what's thrown in Java is cleared, as nothing here can catch it.
        unsafe {
            let get_env: unsafe extern "C" fn(*mut *const *const c_void, *mut Env, i32) -> i32 =
                std::mem::transmute_copy(&*(*vm).add(GET_ENV));
            let attached_here = get_env(vm, &raw mut env, JNI_VERSION_1_6) != 0;
            if attached_here {
                let attach: unsafe extern "C" fn(*mut *const *const c_void, *mut Env, *mut c_void) -> i32 =
                    std::mem::transmute_copy(&*(*vm).add(ATTACH_CURRENT_THREAD));
                if attach(vm, &raw mut env, std::ptr::null_mut()) != 0 || env.is_null() {
                    return;
                }
            }
            call(env, found);
            if jni::<unsafe extern "C" fn(Env) -> u8>(env, EXCEPTION_CHECK)(env) != 0 {
                jni::<unsafe extern "C" fn(Env)>(env, EXCEPTION_CLEAR)(env);
            }
            if attached_here {
                let detach: unsafe extern "C" fn(*mut *const *const c_void) -> i32 =
                    std::mem::transmute_copy(&*(*vm).add(DETACH_CURRENT_THREAD));
                detach(vm);
            }
        }
    }

    /// A new Java string, which the caller lets go of.
    ///
    /// # Safety
    /// `env` is this thread's live environment.
    unsafe fn java_string(env: Env, text: &str) -> Object {
        let units: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: as the caller promises; the characters last through the call.
        unsafe { jni::<unsafe extern "C" fn(Env, *const u16, i32) -> Object>(env, NEW_STRING)(env, units.as_ptr(), units.len() as i32) }
    }

    /// Lets go of a local reference, as a thread that stays in native code never gets back to Java to have them let go of.
    ///
    /// # Safety
    /// `env` is this thread's live environment, and `object` is a local reference in it, or null.
    unsafe fn let_go(env: Env, object: Object) {
        if !object.is_null() {
            // SAFETY: as the caller promises.
            unsafe { jni::<unsafe extern "C" fn(Env, Object)>(env, DELETE_LOCAL_REF)(env, object) };
        }
    }

    fn show_popup(id: u32, kind: &str, json: &str) {
        with_env(|env, found| {
            // SAFETY: the environment is this thread's, the method takes an int and two strings, and the strings are let
            // go of after.
            unsafe {
                let (kind, json) = (java_string(env, kind), java_string(env, json));
                let arguments = [u64::from(id), kind as u64, json as u64];
                jni::<unsafe extern "C" fn(Env, Object, usize, *const u64)>(env, CALL_VOID_METHOD_A)(
                    env,
                    found.activity as Object,
                    found.popup,
                    arguments.as_ptr(),
                );
                let_go(env, kind);
                let_go(env, json);
            }
        });
    }

    /// The colors at the top and bottom edges of what's drawn, as ARGB, or 0, for the system's bars beside them.
    fn bar_colors(top: u32, bottom: u32) {
        with_env(|env, found| {
            // SAFETY: the environment is this thread's, and the method takes two ints.
            unsafe {
                let arguments = [u64::from(top), u64::from(bottom)];
                jni::<unsafe extern "C" fn(Env, Object, usize, *const u64)>(env, CALL_VOID_METHOD_A)(
                    env,
                    found.activity as Object,
                    found.bar_colors,
                    arguments.as_ptr(),
                );
            }
        });
    }

    fn show_toast(text: &str, long: bool) {
        with_env(|env, found| {
            // SAFETY: as in `show_popup`, for a string and a boolean.
            unsafe {
                let text = java_string(env, text);
                let arguments = [text as u64, u64::from(long)];
                jni::<unsafe extern "C" fn(Env, Object, usize, *const u64)>(env, CALL_VOID_METHOD_A)(
                    env,
                    found.activity as Object,
                    found.toast,
                    arguments.as_ptr(),
                );
                let_go(env, text);
            }
        });
    }

    /// A popup's been answered: see `rsui::popups::android::answered` for what it says.
    ///
    /// # Safety
    /// Java calls this with the popup's number and its answer, or null.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn Java_dev_rsui_RsuiActivity_nativePopupAnswered(env: Env, _class: Object, id: i32, answer: Object) {
        // SAFETY: as Java promises.
        let answer = (!answer.is_null()).then(|| unsafe { string(env, answer) });
        rsui::popups::android::answered(id as u32, answer);
    }

    /// The activity's been made.
    ///
    /// # Safety
    /// Java calls this, with the activity's environment and what `RsuiActivity.nativeCreate` declares.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn Java_dev_rsui_RsuiActivity_nativeCreate(
        env: Env,
        _class: Object,
        activity: Object,
        assets: Object,
        files: Object,
        _cache: Object,
        density: f32,
        night: u8,
    ) {
        // SAFETY: as Java promises; what's kept is a global reference, which lasts.
        unsafe {
            if ACTIVITY.get().is_none() {
                let mut vm: *mut c_void = std::ptr::null_mut();
                jni::<unsafe extern "C" fn(Env, *mut *mut c_void) -> i32>(env, GET_JAVA_VM)(env, &raw mut vm);
                let kept = jni::<unsafe extern "C" fn(Env, Object) -> Object>(env, NEW_GLOBAL_REF)(env, activity);
                let class = jni::<unsafe extern "C" fn(Env, Object) -> Object>(env, GET_OBJECT_CLASS)(env, activity);
                let method = |name: &std::ffi::CStr, signature: &std::ffi::CStr| {
                    jni::<unsafe extern "C" fn(Env, Object, *const c_char, *const c_char) -> usize>(env, GET_METHOD_ID)(
                        env,
                        class,
                        name.as_ptr(),
                        signature.as_ptr(),
                    )
                };
                let show_keyboard = method(c"showKeyboard", c"(Z)V");
                let finish = method(c"finish", c"()V");
                let popup = method(c"popup", c"(ILjava/lang/String;Ljava/lang/String;)V");
                let toast = method(c"toast", c"(Ljava/lang/String;Z)V");
                let bars = method(c"barColors", c"(II)V");
                jni::<unsafe extern "C" fn(Env, Object)>(env, DELETE_LOCAL_REF)(env, class);
                let _ = ACTIVITY.set(Activity { vm: vm as usize, activity: kept as usize, show_keyboard, finish, popup, toast, bar_colors: bars });
                rsui::android::on_bar_colors(bar_colors);
                rsui::popups::android::set_activity(rsui::popups::android::Activity { show: show_popup, toast: show_toast });
            }
            let manager = AAssetManager_fromJava(env, assets);
            let data = std::path::PathBuf::from(string(env, files));
            rsui::android::start(run, data, manager, density, night != 0, show_keyboard);
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeConfiguration(_env: Env, _class: Object, density: f32, night: u8) {
        rsui::android::configuration(density, night != 0);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeShown(_env: Env, _class: Object, shown: u8) {
        rsui::android::shown(shown != 0);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeFocus(_env: Env, _class: Object, focused: u8) {
        rsui::android::focus(focused != 0);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeLowMemory(_env: Env, _class: Object) {
        rsui::android::low_memory();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeDestroy(_env: Env, _class: Object) {
        rsui::android::destroy();
    }

    /// # Safety
    /// Java calls this with the activity's surface, or null once it's gone.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn Java_dev_rsui_RsuiActivity_nativeSurface(env: Env, _class: Object, surface: Object) {
        let window = if surface.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: a live surface; the window that comes back is acquired, for the app's thread to let go of.
            unsafe { ANativeWindow_fromSurface(env, surface) }
        };
        rsui::android::surface(window);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeRedraw(_env: Env, _class: Object) {
        rsui::android::redraw();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeTouch(_env: Env, _class: Object, action: i32, pointer: i32, x: f32, y: f32) {
        rsui::android::touch(action, pointer, x, y);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeKey(
        _env: Env,
        _class: Object,
        code: i32,
        meta: i32,
        down: u8,
        repeat: i32,
        unicode: i32,
    ) {
        rsui::android::key(code, meta, down != 0, repeat, unicode);
    }

    /// # Safety
    /// Java calls this with the text being composed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn Java_dev_rsui_RsuiActivity_nativeCompose(env: Env, _class: Object, text: Object) {
        // SAFETY: as Java promises.
        rsui::android::compose(unsafe { string(env, text) });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeFinishComposing(_env: Env, _class: Object) {
        rsui::android::finish_composing();
    }

    /// # Safety
    /// Java calls this with the text the keyboard typed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn Java_dev_rsui_RsuiActivity_nativeCommit(env: Env, _class: Object, text: Object) {
        // SAFETY: as Java promises.
        rsui::android::commit(unsafe { string(env, text) });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn Java_dev_rsui_RsuiActivity_nativeDelete(_env: Env, _class: Object, before: i32, after: i32) {
        rsui::android::delete(before, after);
    }

    /// Which package is installed, as the APK the app's library is loaded from (it's mapped from inside it), its size and
    /// when it was written, which an update changes. None if it can't be told, which has everything made again each time.
    fn installed_package() -> Option<String> {
        let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
        let apk = maps.lines().filter_map(|line| line.split_whitespace().nth(5)).find(|path| path.ends_with("/base.apk"))?;
        let found = std::fs::metadata(apk).ok()?;
        let written = found.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
        Some(format!("{apk} {} {written}", found.len()))
    }

    /// The app's thread: the files out of the APK, then the app. Once it's done, so is the activity, and the process, so
    /// that an app started again starts from nothing.
    fn run() {
        let Some(data) = rsui::android::data_folder() else {
            eprintln!("rsui: the activity has no folder of its own");
            return;
        };
        // An app can't write to the system's temporary folder, and has no home, so both are its own folders.
        let temp = data.join("tmp");
        let _ = std::fs::create_dir_all(&temp);
        // SAFETY: nothing else reads the environment yet; the threads there are only Android's and the log's.
        unsafe {
            std::env::set_var("TMPDIR", &temp);
            std::env::set_var("HOME", &data);
        }
        // An app on Android is started with no environment of its own, so what rsui's switches would be set to
        // (RSUI_JIT, RSUI_STARTUP_TIMES, ...) can be written, a `NAME=value` a line, to `env` in its folder:
        // `adb shell run-as <id> sh -c 'echo RSUI_JIT=eager > files/env'`, in a build that's debuggable.
        if let Ok(said) = std::fs::read_to_string(data.join("env")) {
            for line in said.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
                if let Some((name, value)) = line.split_once('=') {
                    eprintln!("rsui: {name}={value}, from {}", data.join("env").display());
                    // SAFETY: as above.
                    unsafe { std::env::set_var(name.trim(), value.trim()) };
                }
            }
        }
        rsui::mark("android thread");
        let folder = data.join("app");
        // What was copied out of the package, and what the app's code compiled to, are good for as long as the package
        // that's installed is the one they came from: an update copies every file again, and makes the cache again.
        let installed = installed_package();
        let installed_file = data.join("installed");
        let cache = data.join("code.cache");
        let same = installed.is_some() && std::fs::read_to_string(&installed_file).ok() == installed && folder.is_dir();
        if !same {
            let _ = std::fs::remove_file(&cache);
            match rsui::android::copy_assets(&folder, true) {
                Ok(copied) => {
                    eprintln!("rsui: {copied} files copied to {}", folder.display());
                    if let Some(installed) = &installed {
                        let _ = std::fs::write(&installed_file, installed);
                    }
                }
                Err(error) => eprintln!("rsui: the app's files couldn't be copied: {error}"),
            }
        }
        rsui::mark("files ready");
        // What the GPU's driver compiled rsui's shaders to, which it checks is its own.
        rsui::render::set_pipeline_cache_file(data.join("pipelines.cache"));
        let mut options = rsui_tsx::Options::default();
        if same && cache.is_file() {
            rsui_js::use_code_cache_file(&cache, &folder);
        } else if installed.is_some() {
            options.write_code_cache = Some((cache, folder.clone()));
        }
        let entry = ["app.tsx", "app.jsx", "app.ts", "app.js", "main.tsx"]
            .into_iter()
            .map(|name| folder.join(name))
            .find(|path| path.is_file());
        if let Some(entry) = entry {
            if let Err(error) = std::env::set_current_dir(&folder) {
                eprintln!("rsui: {error}");
            }
            if let Err(error) = rsui_tsx::run(&entry, options) {
                eprintln!("rsui: {error:?}");
            }
        } else {
            eprintln!("rsui: the app has no app.tsx");
        }
        finish();
        std::process::exit(0);
    }
}
