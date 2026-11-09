#![allow(unexpected_cfgs)]

use gpui::Window;

#[cfg(target_os = "macos")]
use objc::{class, msg_send, runtime::Object, sel, sel_impl};
#[cfg(target_os = "macos")]
use objc_foundation::{INSString, NSString};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[cfg(target_os = "macos")]
#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
pub struct NativeWebView {
    webview: usize,
    configuration: usize,
}

#[cfg(target_os = "windows")]
pub struct NativeWebView {
    webview: wry::WebView,
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub struct NativeWebView;

#[cfg(target_os = "macos")]
pub fn attach(window: &Window, url: &str) -> Result<NativeWebView, String> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| format!("Не удалось получить native-окно: {error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("Окно не является AppKit-окном".to_owned());
    };
    let native_view = handle.ns_view.as_ptr() as *mut Object;
    unsafe {
        let configuration: *mut Object = msg_send![class!(WKWebViewConfiguration), new];
        let mut frame: core_graphics_types::geometry::CGRect = msg_send![native_view, bounds];
        frame.origin.x = f64::from(crate::SIDEBAR_WIDTH + crate::CONTENT_MARGIN);
        frame.origin.y = f64::from(crate::CONTENT_MARGIN);
        frame.size.width = (frame.size.width
            - f64::from(crate::SIDEBAR_WIDTH + crate::CONTENT_MARGIN * 2.0))
        .max(0.0);
        frame.size.height = (frame.size.height
            - f64::from(crate::HEADER_HEIGHT + crate::CONTENT_MARGIN * 2.0))
        .max(0.0);
        let webview: *mut Object = msg_send![class!(WKWebView), alloc];
        let webview: *mut Object =
            msg_send![webview, initWithFrame: frame configuration: configuration];
        let url = NSString::from_str(url);
        let url: *mut Object = msg_send![class!(NSURL), URLWithString: url];
        let request: *mut Object = msg_send![class!(NSURLRequest), requestWithURL: url];
        let _: () = msg_send![webview, loadRequest: request];
        let _: () = msg_send![webview, setAutoresizingMask: 18usize];
        let _: () = msg_send![native_view, addSubview: webview];
        Ok(NativeWebView {
            webview: webview as usize,
            configuration: configuration as usize,
        })
    }
}

#[cfg(target_os = "windows")]
pub fn attach(window: &Window, url: &str) -> Result<NativeWebView, String> {
    let webview = wry::WebViewBuilder::new()
        .with_url(url)
        .with_bounds(video_bounds(window))
        .build_as_child(window)
        .map_err(|error| format!("Не удалось запустить WebView2: {error}"))?;
    Ok(NativeWebView { webview })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn attach(_window: &Window, _url: &str) -> Result<NativeWebView, String> {
    Err("Встроенный просмотр поддерживается только в macOS и Windows".to_owned())
}

#[cfg(target_os = "macos")]
pub fn detach(view: NativeWebView) {
    unsafe {
        let webview = view.webview as *mut Object;
        let configuration = view.configuration as *mut Object;
        let _: () = msg_send![webview, removeFromSuperview];
        let _: () = msg_send![webview, release];
        let _: () = msg_send![configuration, release];
    }
}

#[cfg(target_os = "windows")]
pub fn detach(view: NativeWebView) {
    drop(view.webview);
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn detach(_view: NativeWebView) {}

#[cfg(target_os = "windows")]
pub fn resize(view: &NativeWebView, window: &Window) {
    let _ = view.webview.set_bounds(video_bounds(window));
}

#[cfg(not(target_os = "windows"))]
pub fn resize(_view: &NativeWebView, _window: &Window) {}

#[cfg(target_os = "windows")]
fn video_bounds(window: &Window) -> wry::Rect {
    use wry::dpi::{LogicalPosition, LogicalSize};

    let size = window.viewport_size();
    let width = f32::from(size.width);
    let height = f32::from(size.height);
    wry::Rect {
        position: LogicalPosition::new(
            crate::SIDEBAR_WIDTH + crate::CONTENT_MARGIN,
            crate::HEADER_HEIGHT + crate::CONTENT_MARGIN,
        )
        .into(),
        size: LogicalSize::new(
            (width - crate::SIDEBAR_WIDTH - crate::CONTENT_MARGIN * 2.0).max(0.0),
            (height - crate::HEADER_HEIGHT - crate::CONTENT_MARGIN * 2.0).max(0.0),
        )
        .into(),
    }
}
