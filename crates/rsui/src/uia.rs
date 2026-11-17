//! Tells Windows UI Automation about the app, which is how screen readers and other assistive
//! technology on Windows find out what's on screen and do things to it.
//!
//! Nothing here runs unless something asks, and the system's library for this is only loaded then.

// The system's own names for all of this, which these match, are the clearest way to refer to them.
#![allow(non_upper_case_globals, non_snake_case)]

use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use rsui_core::{ActionKind, Role};
use windows::Win32::Foundation::{E_FAIL, HMODULE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{BOOL, BSTR, Error, IUnknown, Interface, PCSTR, PCWSTR, Result, s, w};
use windows_core::{IUnknownImpl, implement};

use crate::access::{AccessCommand, AccessNode, AccessTree};

/// Tells the window that there are commands to carry out.
pub(crate) const ACCESS_COMMAND: u32 = WM_APP + 3;

/// What this says for a runtime id it makes up itself.
const APPEND_RUNTIME_ID: i32 = 3;
/// What's in the error for something that isn't there any more.
const ELEMENT_NOT_AVAILABLE: i32 = 0x8004_0201u32 as i32;

type ReturnProvider = unsafe extern "system" fn(HWND, WPARAM, LPARAM, *mut c_void) -> LRESULT;
type HostFromWindow = unsafe extern "system" fn(HWND, *mut *mut c_void) -> i32;
type RaiseEvent = unsafe extern "system" fn(*mut c_void, i32) -> i32;
type RaiseStructure = unsafe extern "system" fn(*mut c_void, i32, *mut i32, i32) -> i32;
type ClientsListening = unsafe extern "system" fn() -> BOOL;

/// The system's functions for this, loaded the first time something asks.
struct Core {
    return_provider: ReturnProvider,
    host_from_window: HostFromWindow,
    raise_event: RaiseEvent,
    raise_structure: RaiseStructure,
    clients_listening: ClientsListening,
}

/// One of the system's functions, which has to be the kind of function `F` is.
///
/// # Safety
///
/// `F` must be a function pointer with the signature the system documents for `name`.
unsafe fn load<F: Copy>(library: HMODULE, name: PCSTR) -> Option<F> {
    // SAFETY: Whether this is what it's said to be is up to the caller.
    unsafe {
        let found = GetProcAddress(library, name)?;
        Some(std::mem::transmute_copy(&found))
    }
}

fn core() -> Option<&'static Core> {
    static CORE: OnceLock<Option<Core>> = OnceLock::new();
    CORE.get_or_init(|| {
        // SAFETY: These are the signatures the system documents for these functions.
        unsafe {
            let library = LoadLibraryW(w!("UIAutomationCore.dll")).ok()?;
            Some(Core {
                return_provider: load(library, s!("UiaReturnRawElementProvider"))?,
                host_from_window: load(library, s!("UiaHostProviderFromHwnd"))?,
                raise_event: load(library, s!("UiaRaiseAutomationEvent"))?,
                raise_structure: load(library, s!("UiaRaiseStructureChangedEvent"))?,
                clients_listening: load(library, s!("UiaClientsAreListening"))?,
            })
        }
    })
    .as_ref()
}

/// What's shared with whichever threads the system asks from.
pub(crate) struct Shared {
    pub(crate) tree: RwLock<AccessTree>,
    pub(crate) commands: Mutex<Vec<(u64, AccessCommand)>>,
    /// The window, as a number, since a handle can't be shared but is fine to use from any thread.
    window: isize,
}

impl Shared {
    pub(crate) fn new(window: HWND, tree: AccessTree) -> Arc<Self> {
        Arc::new(Self {
            tree: RwLock::new(tree),
            commands: Mutex::new(Vec::new()),
            window: window.0 as isize,
        })
    }

    fn window(&self) -> HWND {
        HWND(self.window as *mut c_void)
    }

    fn send(&self, id: u64, command: AccessCommand) -> Result<()> {
        self.commands
            .lock()
            .map_err(|_| Error::from(E_FAIL))?
            .push((id, command));
        // SAFETY: Posting a message to a window has no preconditions.
        unsafe { PostMessageW(Some(self.window()), ACCESS_COMMAND, WPARAM(0), LPARAM(0)) }
    }

    /// Where the window's top left corner is on the screen.
    fn corner(&self) -> (f32, f32) {
        let mut corner = POINT::default();
        // SAFETY: `corner` is a valid point to convert, and a window that's gone just leaves it as it is.
        unsafe {
            let _ = ClientToScreen(self.window(), &mut corner);
        }
        (corner.x as f32, corner.y as f32)
    }
}

fn gone() -> Error {
    Error::from_hresult(windows::core::HRESULT(ELEMENT_NOT_AVAILABLE))
}

/// One node, to the system. These are made as they're asked for, and only hold its id.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    IInvokeProvider,
    IToggleProvider,
    IValueProvider,
    IScrollProvider,
    IScrollItemProvider
)]
struct Provider {
    shared: Arc<Shared>,
    id: u64,
}

/// A provider for a node, as the one interface the system needs to start with.
fn fragment(shared: &Arc<Shared>, id: u64) -> IRawElementProviderFragment {
    Provider {
        shared: shared.clone(),
        id,
    }
    .into()
}

/// The provider for the whole window.
pub(crate) fn root_provider(shared: &Arc<Shared>) -> Option<IRawElementProviderSimple> {
    let id = shared.tree.read().ok()?.nodes.first()?.id;
    fragment(shared, id).cast().ok()
}

/// Hands the system what it asked for when it asked the window.
pub(crate) fn return_provider(
    window: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    provider: &IRawElementProviderSimple,
) -> Option<LRESULT> {
    let core = core()?;
    // SAFETY: These are the arguments this was asked with, and the provider is valid.
    Some(unsafe { (core.return_provider)(window, wparam, lparam, provider.as_raw()) })
}

/// Tells the system that the keyboard's moved, if anyone's listening.
pub(crate) fn raise_focus_changed(shared: &Arc<Shared>, id: u64) {
    let Some(core) = core() else {
        return;
    };
    // SAFETY: The provider is valid for the call, and this has no other preconditions.
    unsafe {
        if !(core.clients_listening)().as_bool() {
            return;
        }
        let provider: Result<IRawElementProviderSimple> = fragment(shared, id).cast();
        if let Ok(provider) = provider {
            (core.raise_event)(provider.as_raw(), UIA_AutomationFocusChangedEventId.0);
        }
    }
}

/// Tells the system that what's in the tree has changed, if anyone's listening.
pub(crate) fn raise_structure_changed(shared: &Arc<Shared>) {
    let Some(core) = core() else {
        return;
    };
    // SAFETY: The provider and id are valid for the call, and it doesn't keep them.
    unsafe {
        if !(core.clients_listening)().as_bool() {
            return;
        }
        let Some(root) = root_provider(shared) else {
            return;
        };
        let mut no_id = [0i32; 1];
        (core.raise_structure)(
            root.as_raw(),
            StructureChangeType_ChildrenInvalidated.0,
            no_id.as_mut_ptr(),
            0,
        );
    }
}

impl Provider_Impl {
    fn with_node<R>(&self, read: impl FnOnce(&AccessTree, &AccessNode) -> R) -> Result<R> {
        let tree = self.shared.tree.read().map_err(|_| Error::from(E_FAIL))?;
        let node = tree.find(self.id).ok_or_else(gone)?;
        Ok(read(&tree, node))
    }

    fn to(&self, id: u64) -> IRawElementProviderFragment {
        fragment(&self.shared, id)
    }

    fn pattern<I: Interface>(&self) -> Result<IUnknown> {
        let object = self.to_object();
        object.cast::<I>()?.cast()
    }

    /// Runs a command for this node, which the window's thread gets to when it can.
    fn command(&self, command: AccessCommand) -> Result<()> {
        self.with_node(|_, _| ())?;
        self.shared.send(self.id, command)
    }

    fn scroll_info(&self) -> Result<rsui_layout::ScrollInfo> {
        self.with_node(|_, node| node.scroll)?
            .ok_or_else(|| Error::from(E_FAIL))
    }

    fn is_vertical(&self) -> Result<bool> {
        let info = self.scroll_info()?;
        Ok(info.content.height > info.view.height + 0.5
            || info.content.width <= info.view.width + 0.5)
    }
}

fn control_type(node: &AccessNode) -> UIA_CONTROLTYPE_ID {
    match node.role {
        Role::Text => UIA_TextControlTypeId,
        Role::Button => UIA_ButtonControlTypeId,
        Role::Link => UIA_HyperlinkControlTypeId,
        Role::TextInput => UIA_EditControlTypeId,
        Role::Checkbox => UIA_CheckBoxControlTypeId,
        Role::List => UIA_ListControlTypeId,
        Role::ListItem => UIA_ListItemControlTypeId,
        Role::Image => UIA_ImageControlTypeId,
        Role::Dialog => UIA_WindowControlTypeId,
        Role::Menu => UIA_MenuControlTypeId,
        Role::MenuItem => UIA_MenuItemControlTypeId,
        Role::Window | Role::Group => UIA_GroupControlTypeId,
        _ => UIA_CustomControlTypeId,
    }
}

impl IRawElementProviderSimple_Impl for Provider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let (invoke, toggle, value, scroll, scroll_item) = self.with_node(|_, node| {
            (
                node.offers(ActionKind::Click),
                node.offers(ActionKind::Toggle),
                node.role == Role::TextInput,
                node.offers(ActionKind::Scroll) && node.scroll.is_some(),
                node.parent.is_some(),
            )
        })?;
        match pattern {
            UIA_InvokePatternId if invoke => self.pattern::<IInvokeProvider>(),
            UIA_TogglePatternId if toggle => self.pattern::<IToggleProvider>(),
            UIA_ValuePatternId if value => self.pattern::<IValueProvider>(),
            UIA_ScrollPatternId if scroll => self.pattern::<IScrollProvider>(),
            UIA_ScrollItemPatternId if scroll_item => self.pattern::<IScrollItemProvider>(),
            // Nothing's the answer, which only reads like an error here.
            _ => Err(Error::empty()),
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        self.with_node(|tree, node| match property {
            UIA_ControlTypePropertyId => VARIANT::from(control_type(node).0),
            // With nothing to say, the window's own title gets to speak for it.
            UIA_NamePropertyId if node.name.is_empty() => VARIANT::default(),
            UIA_NamePropertyId => VARIANT::from(BSTR::from(node.name.as_str())),
            UIA_AutomationIdPropertyId => node
                .key
                .as_deref()
                .map_or_else(VARIANT::default, |key| VARIANT::from(BSTR::from(key))),
            UIA_HelpTextPropertyId => node.help.as_ref().map_or_else(VARIANT::default, |help| {
                VARIANT::from(BSTR::from(help.as_str()))
            }),
            UIA_FrameworkIdPropertyId => VARIANT::from(BSTR::from("rsui")),
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(node.focusable()),
            UIA_HasKeyboardFocusPropertyId => {
                let at = tree.focused.and_then(|at| tree.nodes.get(at as usize));
                VARIANT::from(at.is_some_and(|focused| focused.id == node.id))
            }
            UIA_IsEnabledPropertyId => VARIANT::from(true),
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => {
                VARIANT::from(node.meaningful)
            }
            UIA_IsOffscreenPropertyId => VARIANT::from(node.offscreen),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        let is_root = self.with_node(|_, node| node.parent.is_none())?;
        if !is_root {
            return Err(Error::empty());
        }
        let core = core().ok_or_else(|| Error::from(E_FAIL))?;
        let mut host = std::ptr::null_mut();
        // SAFETY: `host` is written by this call, which hands over one reference.
        unsafe {
            windows::core::HRESULT((core.host_from_window)(self.shared.window(), &mut host))
                .ok()?;
            Ok(IRawElementProviderSimple::from_raw(host))
        }
    }
}

impl IRawElementProviderFragment_Impl for Provider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let target = self.with_node(|tree, node| match direction {
            NavigateDirection_Parent => node.parent.map(|parent| tree.nodes[parent as usize].id),
            NavigateDirection_FirstChild => node
                .children
                .first()
                .map(|child| tree.nodes[*child as usize].id),
            NavigateDirection_LastChild => node
                .children
                .last()
                .map(|child| tree.nodes[*child as usize].id),
            NavigateDirection_NextSibling | NavigateDirection_PreviousSibling => {
                let parent = &tree.nodes[node.parent? as usize];
                let at = parent
                    .children
                    .iter()
                    .position(|child| tree.nodes[*child as usize].id == node.id)?;
                let next = if direction == NavigateDirection_NextSibling {
                    at.checked_add(1)
                } else {
                    at.checked_sub(1)
                }?;
                parent
                    .children
                    .get(next)
                    .map(|child| tree.nodes[*child as usize].id)
            }
            _ => None,
        })?;
        match target {
            Some(id) => Ok(self.to(id)),
            None => Err(Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // Only the root's comes from the window, as the system works that one out for itself.
        if self.with_node(|_, node| node.parent.is_none())? {
            return Err(Error::empty());
        }
        let parts = [APPEND_RUNTIME_ID, (self.id >> 32) as i32, self.id as i32];
        // SAFETY: The array has room for each of these, which it copies.
        unsafe {
            let array = SafeArrayCreateVector(VT_I4, 0, parts.len() as u32);
            if array.is_null() {
                return Err(Error::from(E_FAIL));
            }
            for (index, part) in parts.iter().enumerate() {
                let index = index as i32;
                SafeArrayPutElement(array, &raw const index, std::ptr::from_ref(part).cast())?;
            }
            Ok(array)
        }
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let rect = self.with_node(|_, node| node.rect)?;
        let (left, top) = self.shared.corner();
        Ok(UiaRect {
            left: f64::from(rect[0] + left),
            top: f64::from(rect[1] + top),
            width: f64::from(rect[2]),
            height: f64::from(rect[3]),
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        self.command(AccessCommand::Focus)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        let root = self.with_node(|tree, _| tree.nodes.first().map(|root| root.id))?;
        let root = root.ok_or_else(gone)?;
        self.to(root).cast()
    }
}

impl IRawElementProviderFragmentRoot_Impl for Provider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let (left, top) = self.shared.corner();
        let found = self.with_node(|tree, _| {
            tree.at_point(x as f32 - left, y as f32 - top)
                .map(|node| node.id)
        })?;
        match found {
            Some(id) => Ok(self.to(id)),
            None => Err(Error::empty()),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let focused = self.with_node(|tree, _| {
            tree.focused
                .and_then(|at| tree.nodes.get(at as usize))
                .map(|node| node.id)
        })?;
        match focused {
            Some(id) => Ok(self.to(id)),
            None => Err(Error::empty()),
        }
    }
}

impl IInvokeProvider_Impl for Provider_Impl {
    fn Invoke(&self) -> Result<()> {
        self.command(AccessCommand::Invoke)
    }
}

impl IToggleProvider_Impl for Provider_Impl {
    fn Toggle(&self) -> Result<()> {
        self.command(AccessCommand::Toggle)
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        let checked = self.with_node(|_, node| node.checked)?;
        Ok(match checked {
            Some(true) => ToggleState_On,
            Some(false) | None => ToggleState_Off,
        })
    }
}

impl IValueProvider_Impl for Provider_Impl {
    fn SetValue(&self, value: &PCWSTR) -> Result<()> {
        // SAFETY: The system gives a valid string, for the length of the call.
        let text = unsafe { value.to_string() }.map_err(|_| Error::from(E_FAIL))?;
        self.command(AccessCommand::SetValue(text))
    }

    fn Value(&self) -> Result<BSTR> {
        let value = self.with_node(|_, node| node.value.clone())?;
        Ok(BSTR::from(
            value.as_ref().map_or("", |value| value.as_str()),
        ))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        let can_change = self.with_node(|_, node| node.offers(ActionKind::SetValue))?;
        Ok((!can_change).into())
    }
}

/// What UI Automation's percentages come to, for something that's as far along as this. Something
/// with nowhere to go is the value it says means that.
fn percent(moved: f32, furthest: f32) -> f64 {
    if furthest <= 0.5 {
        UIA_ScrollPatternNoScroll
    } else {
        f64::from(moved / furthest * 100.0).clamp(0.0, 100.0)
    }
}

impl IScrollProvider_Impl for Provider_Impl {
    fn Scroll(&self, horizontal: ScrollAmount, vertical: ScrollAmount) -> Result<()> {
        let amount = if self.is_vertical()? {
            vertical
        } else {
            horizontal
        };
        let command = match amount {
            ScrollAmount_LargeDecrement => AccessCommand::ScrollPages(-1),
            ScrollAmount_LargeIncrement => AccessCommand::ScrollPages(1),
            ScrollAmount_SmallDecrement => AccessCommand::ScrollSteps(-1),
            ScrollAmount_SmallIncrement => AccessCommand::ScrollSteps(1),
            _ => return Ok(()),
        };
        self.command(command)
    }

    fn SetScrollPercent(&self, horizontal: f64, vertical: f64) -> Result<()> {
        let wanted = if self.is_vertical()? {
            vertical
        } else {
            horizontal
        };
        // This number means leaving it as it is.
        if wanted == UIA_ScrollPatternNoScroll {
            return Ok(());
        }
        self.command(AccessCommand::ScrollTo((wanted / 100.0).clamp(0.0, 1.0)))
    }

    fn HorizontalScrollPercent(&self) -> Result<f64> {
        let info = self.scroll_info()?;
        Ok(percent(info.x, info.furthest_x()))
    }

    fn VerticalScrollPercent(&self) -> Result<f64> {
        let info = self.scroll_info()?;
        Ok(percent(info.y, info.furthest_y()))
    }

    fn HorizontalViewSize(&self) -> Result<f64> {
        let info = self.scroll_info()?;
        Ok(f64::from(info.view.width / info.content.width.max(1.0) * 100.0).min(100.0))
    }

    fn VerticalViewSize(&self) -> Result<f64> {
        let info = self.scroll_info()?;
        Ok(f64::from(info.view.height / info.content.height.max(1.0) * 100.0).min(100.0))
    }

    fn HorizontallyScrollable(&self) -> Result<BOOL> {
        Ok((self.scroll_info()?.furthest_x() > 0.5).into())
    }

    fn VerticallyScrollable(&self) -> Result<BOOL> {
        Ok((self.scroll_info()?.furthest_y() > 0.5).into())
    }
}

impl IScrollItemProvider_Impl for Provider_Impl {
    fn ScrollIntoView(&self) -> Result<()> {
        self.command(AccessCommand::Reveal)
    }
}
