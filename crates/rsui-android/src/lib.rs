//! An rsui app on Android. Android starts this library for the app's activity (`NativeActivity`, as the APK's manifest
//! says), which copies the app's files out of the APK into the app's own folder the first time, and runs it there with
//! `rsui-tsx`, in the activity's window, until the activity ends.
//!
//! `tools/android/build-apk.ps1` makes the APK: this library for arm64, the app's files as assets with a list of them in
//! `rsui-assets.txt`, and a manifest.

#[cfg(target_os = "android")]
mod app {
    use std::ffi::c_void;

    #[global_allocator]
    static ALLOCATOR: rsui::UiAlloc = rsui::UiAlloc;

    /// Where Android starts an activity that runs native code.
    ///
    /// # Safety
    /// Android calls this once for each activity it makes, with the activity.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ANativeActivity_onCreate(
        activity: *mut rsui::android::ANativeActivity,
        _saved: *mut c_void,
        _saved_size: usize,
    ) {
        // SAFETY: as the caller promises.
        unsafe { rsui::android::on_create(activity, run) };
    }

    /// The app's thread: the files out of the APK, then the app.
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
        let folder = data.join("app");
        match rsui::android::copy_assets(&folder) {
            Ok(copied) => eprintln!("rsui: {copied} files copied to {}", folder.display()),
            Err(error) => eprintln!("rsui: the app's files couldn't be copied: {error}"),
        }
        let entry = ["app.tsx", "app.jsx", "app.ts", "app.js", "main.tsx"]
            .into_iter()
            .map(|name| folder.join(name))
            .find(|path| path.is_file());
        let Some(entry) = entry else {
            eprintln!("rsui: the app has no app.tsx");
            rsui::android::finish();
            return;
        };
        if let Err(error) = std::env::set_current_dir(&folder) {
            eprintln!("rsui: {error}");
        }
        if let Err(error) = rsui_tsx::run(&entry, rsui_tsx::Options::default()) {
            eprintln!("rsui: {error:?}");
        }
        rsui::android::finish();
    }
}
