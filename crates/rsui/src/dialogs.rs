//! The system's dialogs for choosing files and folders. They wait for someone to answer, so these are for calling on a thread
//! that can wait, and the window carries on drawing meanwhile. Nothing here is loaded until one's asked for.

#[cfg(windows)]
use std::sync::atomic::{AtomicIsize, Ordering};

#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog,
    IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem, SHCreateItemFromParsingName,
    SIGDN_FILESYSPATH,
};
#[cfg(windows)]
use windows::core::{HSTRING, Interface, PCWSTR};

/// What to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    OpenFile,
    SaveFile,
    PickFolder,
}

/// What a dialog says and starts from.
#[derive(Debug, Clone, Default)]
pub struct DialogOptions {
    pub title: Option<String>,
    /// A file or folder to start from.
    pub start: Option<String>,
    /// What kinds of file to show, each as a name and patterns like `*.txt;*.md`.
    pub filters: Vec<(String, String)>,
}

/// Asks someone to choose a file or folder. The system's dialogs aren't written here yet (on Linux they'll come from the
/// desktop's portal), so this says so.
#[cfg(not(windows))]
pub fn ask_for_path(_kind: DialogKind, _options: DialogOptions) -> Result<Option<String>, String> {
    Err("the system's file dialogs aren't on this system yet".to_owned())
}

#[cfg(windows)]
/// The window that's open, for dialogs to belong to.
static CURRENT_WINDOW: AtomicIsize = AtomicIsize::new(0);

#[cfg(windows)]
pub(crate) fn set_current_window(window: HWND) {
    CURRENT_WINDOW.store(window.0 as isize, Ordering::Release);
}

#[cfg(windows)]
pub(crate) fn current_window() -> HWND {
    HWND(CURRENT_WINDOW.load(Ordering::Acquire) as *mut _)
}

#[cfg(windows)]
/// Asks someone to choose a file or folder, and says what they chose, if they did. It goes on a thread of its own, as these
/// dialogs need a lot of stack for what the system loads into them, and the window it belongs to stays disabled until it's
/// answered.
pub fn ask_for_path(kind: DialogKind, options: DialogOptions) -> Result<Option<String>, String> {
    let owner = CURRENT_WINDOW.load(Ordering::Acquire);
    let thread = std::thread::Builder::new()
        .name("rsui dialog".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || ask_on_this_thread(kind, &options, HWND(owner as *mut _)))
        .map_err(|error| error.to_string())?;
    thread
        .join()
        .unwrap_or_else(|_| Err("the dialog stopped working".to_owned()))
}

#[cfg(windows)]
/// [`ask_for_path`], on this thread, with no thought for what else it's holding up.
#[doc(hidden)]
pub fn ask_on_this_thread(
    kind: DialogKind,
    options: &DialogOptions,
    owner: HWND,
) -> Result<Option<String>, String> {
    // SAFETY: this is undone at the end of this function, on this same thread.
    let started =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let result = show(kind, options, owner).map_err(|error| error.to_string());
    if started.is_ok() {
        // SAFETY: as above.
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(windows)]
fn show(
    kind: DialogKind,
    options: &DialogOptions,
    owner: HWND,
) -> windows::core::Result<Option<String>> {
    // SAFETY: these are calls on objects made here, with strings that outlive them. What's returned by `GetDisplayName` is
    // freed once, after being copied.
    unsafe {
        let dialog: IFileDialog = if kind == DialogKind::SaveFile {
            CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?
                .cast()?
        } else {
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?
                .cast()?
        };
        let mut flags = dialog.GetOptions()? | FOS_FORCEFILESYSTEM;
        match kind {
            DialogKind::PickFolder => flags |= FOS_PICKFOLDERS,
            DialogKind::SaveFile => flags |= FOS_OVERWRITEPROMPT,
            DialogKind::OpenFile => {}
        }
        dialog.SetOptions(flags)?;
        if let Some(title) = &options.title {
            dialog.SetTitle(&HSTRING::from(title.as_str()))?;
        }
        // These have to last until the dialog's been shown.
        let filters: Vec<(HSTRING, HSTRING)> = options
            .filters
            .iter()
            .map(|(name, pattern)| {
                (
                    HSTRING::from(name.as_str()),
                    HSTRING::from(pattern.as_str()),
                )
            })
            .collect();
        let specs: Vec<COMDLG_FILTERSPEC> = filters
            .iter()
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            })
            .collect();
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs)?;
        }
        if let Some(start) = &options.start {
            let path = std::path::Path::new(start);
            let (folder, name) = if path.is_dir() {
                (Some(path), None)
            } else {
                (path.parent(), path.file_name())
            };
            if let Some(folder) = folder.filter(|folder| !folder.as_os_str().is_empty()) {
                let folder: Option<IShellItem> =
                    SHCreateItemFromParsingName(&HSTRING::from(folder.as_os_str()), None).ok();
                if let Some(folder) = folder {
                    dialog.SetFolder(&folder)?;
                }
            }
            if let Some(name) = name {
                dialog.SetFileName(&HSTRING::from(name))?;
            }
        }
        if let Err(error) = dialog.Show(Some(owner).filter(|owner| !owner.0.is_null())) {
            return if error.code() == ERROR_CANCELLED.to_hresult() {
                Ok(None)
            } else {
                Err(error)
            };
        }
        let chosen = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = chosen.to_string();
        CoTaskMemFree(Some(chosen.0.cast()));
        Ok(Some(path.map_err(|_| windows::core::Error::empty())?))
    }
}
