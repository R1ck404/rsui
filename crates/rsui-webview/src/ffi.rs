//! The parts of WebView2's COM interfaces this uses, written out from `WebView2.h` (SDK 1.0.3719.77). Each method is at
//! the place in the table the header puts it; ones this never calls are only there to keep the rest in their places, and
//! take nothing, as calling them would be wrong anyway. What's called back, the handlers, are all one object: see
//! [`crate::handler`].

#![allow(non_snake_case, clippy::too_many_arguments)]

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, RECT};
use windows::core::{BOOL, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, PWSTR, interface};

/// A handler, or anything else passed as an interface this doesn't name.
pub(crate) type Raw = *mut c_void;

#[interface("b96d755e-0319-4e92-a296-23436f46a1fc")]
pub unsafe trait ICoreWebView2Environment: IUnknown {
    pub fn CreateCoreWebView2Controller(&self, parent: HWND, handler: Raw) -> HRESULT;
    pub fn CreateWebResourceResponse(&self) -> HRESULT;
    pub fn get_BrowserVersionString(&self, version: *mut PWSTR) -> HRESULT;
}

/// Version 10, with its parents' methods in between as places only, for making a view in a profile of its own.
#[interface("ee0eb9df-6f12-46ce-b53f-3f47b9c928e0")]
pub unsafe trait ICoreWebView2Environment10: IUnknown {
    pub fn CreateCoreWebView2Controller(&self, parent: HWND, handler: Raw) -> HRESULT;
    pub fn CreateWebResourceResponse(&self) -> HRESULT;
    pub fn get_BrowserVersionString(&self, version: *mut PWSTR) -> HRESULT;
    pub fn add_NewBrowserVersionAvailable(&self) -> HRESULT;
    pub fn remove_NewBrowserVersionAvailable(&self) -> HRESULT;
    // 2
    pub fn CreateWebResourceRequest(&self) -> HRESULT;
    // 3
    pub fn CreateCoreWebView2CompositionController(&self) -> HRESULT;
    pub fn CreateCoreWebView2PointerInfo(&self) -> HRESULT;
    // 4
    pub fn GetAutomationProviderForWindow(&self) -> HRESULT;
    // 5
    pub fn add_BrowserProcessExited(&self) -> HRESULT;
    pub fn remove_BrowserProcessExited(&self) -> HRESULT;
    // 6
    pub fn CreatePrintSettings(&self) -> HRESULT;
    // 7
    pub fn get_UserDataFolder(&self) -> HRESULT;
    // 8
    pub fn add_ProcessInfosChanged(&self) -> HRESULT;
    pub fn remove_ProcessInfosChanged(&self) -> HRESULT;
    pub fn GetProcessInfos(&self) -> HRESULT;
    // 9
    pub fn CreateContextMenuItem(&self) -> HRESULT;
    // 10
    pub fn CreateCoreWebView2ControllerOptions(&self, options: *mut Option<ICoreWebView2ControllerOptions>) -> HRESULT;
    pub fn CreateCoreWebView2ControllerWithOptions(
        &self,
        parent: HWND,
        options: *mut c_void,
        handler: Raw,
    ) -> HRESULT;
}

#[interface("12aae616-8ccb-44ec-bcb3-eb1831881635")]
pub unsafe trait ICoreWebView2ControllerOptions: IUnknown {
    pub fn get_ProfileName(&self, value: *mut PWSTR) -> HRESULT;
    pub fn put_ProfileName(&self, value: PCWSTR) -> HRESULT;
    pub fn get_IsInPrivateModeEnabled(&self, value: *mut BOOL) -> HRESULT;
    pub fn put_IsInPrivateModeEnabled(&self, value: BOOL) -> HRESULT;
}

#[interface("4d00c0d1-9434-4eb6-8078-8697a560334f")]
pub unsafe trait ICoreWebView2Controller: IUnknown {
    pub fn get_IsVisible(&self, visible: *mut BOOL) -> HRESULT;
    pub fn put_IsVisible(&self, visible: BOOL) -> HRESULT;
    pub fn get_Bounds(&self, bounds: *mut RECT) -> HRESULT;
    pub fn put_Bounds(&self, bounds: RECT) -> HRESULT;
    pub fn get_ZoomFactor(&self, zoom: *mut f64) -> HRESULT;
    pub fn put_ZoomFactor(&self, zoom: f64) -> HRESULT;
    pub fn add_ZoomFactorChanged(&self) -> HRESULT;
    pub fn remove_ZoomFactorChanged(&self) -> HRESULT;
    pub fn SetBoundsAndZoomFactor(&self, bounds: RECT, zoom: f64) -> HRESULT;
    pub fn MoveFocus(&self, reason: i32) -> HRESULT;
    pub fn add_MoveFocusRequested(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_MoveFocusRequested(&self, token: i64) -> HRESULT;
    pub fn add_GotFocus(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_GotFocus(&self, token: i64) -> HRESULT;
    pub fn add_LostFocus(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_LostFocus(&self, token: i64) -> HRESULT;
    pub fn add_AcceleratorKeyPressed(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_AcceleratorKeyPressed(&self) -> HRESULT;
    pub fn get_ParentWindow(&self, parent: *mut HWND) -> HRESULT;
    pub fn put_ParentWindow(&self, parent: HWND) -> HRESULT;
    pub fn NotifyParentWindowPositionChanged(&self) -> HRESULT;
    pub fn Close(&self) -> HRESULT;
    pub fn get_CoreWebView2(&self, view: *mut Option<ICoreWebView2>) -> HRESULT;
}

#[interface("76eceacb-0462-4d94-ac83-423a6793775e")]
pub unsafe trait ICoreWebView2: IUnknown {
    pub fn get_Settings(&self, settings: *mut Option<ICoreWebView2Settings>) -> HRESULT;
    pub fn get_Source(&self, uri: *mut PWSTR) -> HRESULT;
    pub fn Navigate(&self, uri: PCWSTR) -> HRESULT;
    pub fn NavigateToString(&self, html: PCWSTR) -> HRESULT;
    pub fn add_NavigationStarting(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_NavigationStarting(&self, token: i64) -> HRESULT;
    pub fn add_ContentLoading(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_ContentLoading(&self, token: i64) -> HRESULT;
    pub fn add_SourceChanged(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_SourceChanged(&self, token: i64) -> HRESULT;
    pub fn add_HistoryChanged(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_HistoryChanged(&self, token: i64) -> HRESULT;
    pub fn add_NavigationCompleted(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_NavigationCompleted(&self, token: i64) -> HRESULT;
    pub fn add_FrameNavigationStarting(&self) -> HRESULT;
    pub fn remove_FrameNavigationStarting(&self) -> HRESULT;
    pub fn add_FrameNavigationCompleted(&self) -> HRESULT;
    pub fn remove_FrameNavigationCompleted(&self) -> HRESULT;
    pub fn add_ScriptDialogOpening(&self) -> HRESULT;
    pub fn remove_ScriptDialogOpening(&self) -> HRESULT;
    pub fn add_PermissionRequested(&self) -> HRESULT;
    pub fn remove_PermissionRequested(&self) -> HRESULT;
    pub fn add_ProcessFailed(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_ProcessFailed(&self, token: i64) -> HRESULT;
    pub fn AddScriptToExecuteOnDocumentCreated(&self, script: PCWSTR, handler: Raw) -> HRESULT;
    pub fn RemoveScriptToExecuteOnDocumentCreated(&self, id: PCWSTR) -> HRESULT;
    pub fn ExecuteScript(&self, script: PCWSTR, handler: Raw) -> HRESULT;
    pub fn CapturePreview(&self, format: i32, stream: Raw, handler: Raw) -> HRESULT;
    pub fn Reload(&self) -> HRESULT;
    pub fn PostWebMessageAsJson(&self, json: PCWSTR) -> HRESULT;
    pub fn PostWebMessageAsString(&self, text: PCWSTR) -> HRESULT;
    pub fn add_WebMessageReceived(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_WebMessageReceived(&self, token: i64) -> HRESULT;
    pub fn CallDevToolsProtocolMethod(&self) -> HRESULT;
    pub fn get_BrowserProcessId(&self, id: *mut u32) -> HRESULT;
    pub fn get_CanGoBack(&self, can: *mut BOOL) -> HRESULT;
    pub fn get_CanGoForward(&self, can: *mut BOOL) -> HRESULT;
    pub fn GoBack(&self) -> HRESULT;
    pub fn GoForward(&self) -> HRESULT;
    pub fn GetDevToolsProtocolEventReceiver(&self) -> HRESULT;
    pub fn Stop(&self) -> HRESULT;
    pub fn add_NewWindowRequested(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_NewWindowRequested(&self, token: i64) -> HRESULT;
    pub fn add_DocumentTitleChanged(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_DocumentTitleChanged(&self, token: i64) -> HRESULT;
    pub fn get_DocumentTitle(&self, title: *mut PWSTR) -> HRESULT;
    pub fn AddHostObjectToScript(&self) -> HRESULT;
    pub fn RemoveHostObjectFromScript(&self) -> HRESULT;
    pub fn OpenDevToolsWindow(&self) -> HRESULT;
    pub fn add_ContainsFullScreenElementChanged(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_ContainsFullScreenElementChanged(&self) -> HRESULT;
    pub fn get_ContainsFullScreenElement(&self, on: *mut BOOL) -> HRESULT;
    pub fn add_WebResourceRequested(&self) -> HRESULT;
    pub fn remove_WebResourceRequested(&self) -> HRESULT;
    pub fn AddWebResourceRequestedFilter(&self) -> HRESULT;
    pub fn RemoveWebResourceRequestedFilter(&self) -> HRESULT;
    pub fn add_WindowCloseRequested(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_WindowCloseRequested(&self, token: i64) -> HRESULT;
}

/// Version 2, which says when the page's own document has been read, and has its cookies.
#[interface("9E8F0CF8-E670-4B5E-B2BC-73E061E3184C")]
pub unsafe trait ICoreWebView2_2: ICoreWebView2 {
    pub fn add_WebResourceResponseReceived(&self) -> HRESULT;
    pub fn remove_WebResourceResponseReceived(&self) -> HRESULT;
    pub fn NavigateWithWebResourceRequest(&self) -> HRESULT;
    pub fn add_DOMContentLoaded(&self, handler: Raw, token: *mut i64) -> HRESULT;
    pub fn remove_DOMContentLoaded(&self, token: i64) -> HRESULT;
    pub fn get_CookieManager(&self, manager: *mut Option<ICoreWebView2CookieManager>) -> HRESULT;
}

#[interface("e562e4f0-d7fa-43ac-8d71-c05150499f00")]
pub unsafe trait ICoreWebView2Settings: IUnknown {
    pub fn get_IsScriptEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_IsScriptEnabled(&self, on: BOOL) -> HRESULT;
    pub fn get_IsWebMessageEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_IsWebMessageEnabled(&self, on: BOOL) -> HRESULT;
    pub fn get_AreDefaultScriptDialogsEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_AreDefaultScriptDialogsEnabled(&self, on: BOOL) -> HRESULT;
    pub fn get_IsStatusBarEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_IsStatusBarEnabled(&self, on: BOOL) -> HRESULT;
    pub fn get_AreDevToolsEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_AreDevToolsEnabled(&self, on: BOOL) -> HRESULT;
    pub fn get_AreDefaultContextMenusEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_AreDefaultContextMenusEnabled(&self, on: BOOL) -> HRESULT;
    pub fn get_AreHostObjectsAllowed(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_AreHostObjectsAllowed(&self, on: BOOL) -> HRESULT;
    pub fn get_IsZoomControlEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_IsZoomControlEnabled(&self, on: BOOL) -> HRESULT;
    pub fn get_IsBuiltInErrorPageEnabled(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_IsBuiltInErrorPageEnabled(&self, on: BOOL) -> HRESULT;
}

#[interface("ee9a0f68-f46c-4e32-ac23-ef8cac224d2a")]
pub unsafe trait ICoreWebView2Settings2: ICoreWebView2Settings {
    pub fn get_UserAgent(&self, agent: *mut PWSTR) -> HRESULT;
    pub fn put_UserAgent(&self, agent: PCWSTR) -> HRESULT;
}

#[interface("5b495469-e119-438a-9b18-7604f25f2e49")]
pub unsafe trait ICoreWebView2NavigationStartingEventArgs: IUnknown {
    pub fn get_Uri(&self, uri: *mut PWSTR) -> HRESULT;
    pub fn get_IsUserInitiated(&self, user: *mut BOOL) -> HRESULT;
    pub fn get_IsRedirected(&self, redirected: *mut BOOL) -> HRESULT;
    pub fn get_RequestHeaders(&self) -> HRESULT;
    pub fn get_Cancel(&self, cancel: *mut BOOL) -> HRESULT;
    pub fn put_Cancel(&self, cancel: BOOL) -> HRESULT;
    pub fn get_NavigationId(&self, id: *mut u64) -> HRESULT;
}

#[interface("30d68b7d-20d9-4752-a9ca-ec8448fbb5c1")]
pub unsafe trait ICoreWebView2NavigationCompletedEventArgs: IUnknown {
    pub fn get_IsSuccess(&self, success: *mut BOOL) -> HRESULT;
    pub fn get_WebErrorStatus(&self, status: *mut i32) -> HRESULT;
    pub fn get_NavigationId(&self, id: *mut u64) -> HRESULT;
}

#[interface("31e0e545-1dba-4266-8914-f63848a1f7d7")]
pub unsafe trait ICoreWebView2SourceChangedEventArgs: IUnknown {
    pub fn get_IsNewDocument(&self, new: *mut BOOL) -> HRESULT;
}

#[interface("34acb11c-fc37-4418-9132-f9c21d1eafb9")]
pub unsafe trait ICoreWebView2NewWindowRequestedEventArgs: IUnknown {
    pub fn get_Uri(&self, uri: *mut PWSTR) -> HRESULT;
    pub fn put_NewWindow(&self, view: *mut c_void) -> HRESULT;
    pub fn get_NewWindow(&self, view: *mut *mut c_void) -> HRESULT;
    pub fn put_Handled(&self, handled: BOOL) -> HRESULT;
    pub fn get_Handled(&self, handled: *mut BOOL) -> HRESULT;
    pub fn get_IsUserInitiated(&self, user: *mut BOOL) -> HRESULT;
}

#[interface("0f99a40c-e962-4207-9e92-e3d542eff849")]
pub unsafe trait ICoreWebView2WebMessageReceivedEventArgs: IUnknown {
    pub fn get_Source(&self, source: *mut PWSTR) -> HRESULT;
    pub fn get_WebMessageAsJson(&self, json: *mut PWSTR) -> HRESULT;
    pub fn TryGetWebMessageAsString(&self, text: *mut PWSTR) -> HRESULT;
}

#[interface("2d6aa13b-3839-4a15-92fc-d88b3c0d9c9d")]
pub unsafe trait ICoreWebView2MoveFocusRequestedEventArgs: IUnknown {
    pub fn get_Reason(&self, reason: *mut i32) -> HRESULT;
    pub fn get_Handled(&self, handled: *mut BOOL) -> HRESULT;
    pub fn put_Handled(&self, handled: BOOL) -> HRESULT;
}

/// What a key pressed in a page says, before the page has it.
#[interface("9f760f8a-fb79-42be-9990-7b56900fa9c7")]
pub unsafe trait ICoreWebView2AcceleratorKeyPressedEventArgs: IUnknown {
    /// 0 down, 1 up, 2 down with Alt (a system key), 3 up with Alt.
    pub fn get_KeyEventKind(&self, kind: *mut i32) -> HRESULT;
    pub fn get_VirtualKey(&self, key: *mut u32) -> HRESULT;
    pub fn get_KeyEventLParam(&self, lparam: *mut i32) -> HRESULT;
    pub fn get_PhysicalKeyStatus(&self) -> HRESULT;
    pub fn get_Handled(&self, handled: *mut BOOL) -> HRESULT;
    pub fn put_Handled(&self, handled: BOOL) -> HRESULT;
}

#[interface("8155a9a4-1474-4a86-8cae-151b0fa6b8ca")]
pub unsafe trait ICoreWebView2ProcessFailedEventArgs: IUnknown {
    pub fn get_ProcessFailedKind(&self, kind: *mut i32) -> HRESULT;
}

#[interface("177CD9E7-B6F5-451A-94A0-5D7A3A4C4141")]
pub unsafe trait ICoreWebView2CookieManager: IUnknown {
    pub fn CreateCookie(
        &self,
        name: PCWSTR,
        value: PCWSTR,
        domain: PCWSTR,
        path: PCWSTR,
        cookie: *mut Option<ICoreWebView2Cookie>,
    ) -> HRESULT;
    pub fn CopyCookie(&self) -> HRESULT;
    pub fn GetCookies(&self, uri: PCWSTR, handler: Raw) -> HRESULT;
    pub fn AddOrUpdateCookie(&self, cookie: *mut c_void) -> HRESULT;
    pub fn DeleteCookie(&self, cookie: *mut c_void) -> HRESULT;
    pub fn DeleteCookies(&self, name: PCWSTR, uri: PCWSTR) -> HRESULT;
    pub fn DeleteCookiesWithDomainAndPath(&self, name: PCWSTR, domain: PCWSTR, path: PCWSTR) -> HRESULT;
    pub fn DeleteAllCookies(&self) -> HRESULT;
}

#[interface("f7f6f714-5d2a-43c6-9503-346ece02d186")]
pub unsafe trait ICoreWebView2CookieList: IUnknown {
    pub fn get_Count(&self, count: *mut u32) -> HRESULT;
    pub fn GetValueAtIndex(&self, index: u32, cookie: *mut Option<ICoreWebView2Cookie>) -> HRESULT;
}

#[interface("AD26D6BE-1486-43E6-BF87-A2034006CA21")]
pub unsafe trait ICoreWebView2Cookie: IUnknown {
    pub fn get_Name(&self, name: *mut PWSTR) -> HRESULT;
    pub fn get_Value(&self, value: *mut PWSTR) -> HRESULT;
    pub fn put_Value(&self, value: PCWSTR) -> HRESULT;
    pub fn get_Domain(&self, domain: *mut PWSTR) -> HRESULT;
    pub fn get_Path(&self, path: *mut PWSTR) -> HRESULT;
    pub fn get_Expires(&self, expires: *mut f64) -> HRESULT;
    pub fn put_Expires(&self, expires: f64) -> HRESULT;
    pub fn get_IsHttpOnly(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_IsHttpOnly(&self, on: BOOL) -> HRESULT;
    pub fn get_SameSite(&self, kind: *mut i32) -> HRESULT;
    pub fn put_SameSite(&self, kind: i32) -> HRESULT;
    pub fn get_IsSecure(&self, on: *mut BOOL) -> HRESULT;
    pub fn put_IsSecure(&self, on: BOOL) -> HRESULT;
    pub fn get_IsSession(&self, on: *mut BOOL) -> HRESULT;
}

/// What each handler is, by its interface's id.
pub(crate) mod handlers {
    use windows::core::GUID;

    pub(crate) const ENVIRONMENT_CREATED: GUID = GUID::from_u128(0x4e8a3389_c9d8_4bd2_b6b5_124fee6cc14d);
    pub(crate) const CONTROLLER_CREATED: GUID = GUID::from_u128(0x6c4819f3_c9b7_4260_8127_c9f5bde7f68c);
    pub(crate) const NAVIGATION_STARTING: GUID = GUID::from_u128(0x9adbe429_f36d_432b_9ddc_f8881fbd76e3);
    pub(crate) const NAVIGATION_COMPLETED: GUID = GUID::from_u128(0xd33a35bf_1c49_4f98_93ab_006e0533fe1c);
    pub(crate) const SOURCE_CHANGED: GUID = GUID::from_u128(0x3c067f9f_5388_4772_8b48_79f7ef1ab37c);
    pub(crate) const CONTENT_LOADING: GUID = GUID::from_u128(0x364471e7_f2be_4910_bdba_d72077d51c4b);
    pub(crate) const HISTORY_CHANGED: GUID = GUID::from_u128(0xc79a420c_efd9_4058_9295_3e8b4bcab645);
    pub(crate) const DOCUMENT_TITLE_CHANGED: GUID = GUID::from_u128(0xf5f2b923_953e_4042_9f95_f3a118e1afd4);
    pub(crate) const NEW_WINDOW_REQUESTED: GUID = GUID::from_u128(0xd4c185fe_c81c_4989_97af_2d3fa7ab5651);
    pub(crate) const WEB_MESSAGE_RECEIVED: GUID = GUID::from_u128(0x57213f19_00e6_49fa_8e07_898ea01ecbd2);
    pub(crate) const EXECUTE_SCRIPT_COMPLETED: GUID = GUID::from_u128(0x49511172_cc67_4bca_9923_137112f4c4cc);
    pub(crate) const ADD_SCRIPT_COMPLETED: GUID = GUID::from_u128(0xb99369f3_9b11_47b5_bc6f_8e7895fcea17);
    pub(crate) const MOVE_FOCUS_REQUESTED: GUID = GUID::from_u128(0x69035451_6dc7_4cb8_9bce_b2bd70ad289f);
    pub(crate) const FOCUS_CHANGED: GUID = GUID::from_u128(0x05ea24bd_6452_4926_9014_4b82b498135d);
    pub(crate) const DOM_CONTENT_LOADED: GUID = GUID::from_u128(0x4bac7e9c_199e_49ed_87ed_249303acf019);
    pub(crate) const GET_COOKIES_COMPLETED: GUID = GUID::from_u128(0x5a4f5069_5c15_47c3_8646_f4de1c116670);
    pub(crate) const PROCESS_FAILED: GUID = GUID::from_u128(0x79e0aea4_990b_42d9_aa1d_0fcc2e5bc7f1);
    pub(crate) const WINDOW_CLOSE_REQUESTED: GUID = GUID::from_u128(0x5c19e9e0_092f_486b_affa_ca8231913039);
    pub(crate) const ACCELERATOR_KEY_PRESSED: GUID = GUID::from_u128(0xb29c7e28_fa79_41a8_8e44_65811c76dcb2);
    pub(crate) const FULL_SCREEN_CHANGED: GUID = GUID::from_u128(0xe45d98b1_afef_45be_8baf_6c7728867f73);
    pub(crate) const CAPTURE_PREVIEW_COMPLETED: GUID = GUID::from_u128(0x697e05e9_3d8f_45fa_96f4_8ffe1ededaf5);
}
