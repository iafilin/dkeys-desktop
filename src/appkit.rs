//! Общие AppKit-хелперы для встройки нативных view поверх gpui-окна (macOS).

#![allow(unexpected_cfgs)]

use core_graphics_types::geometry::CGRect;
#[cfg(target_arch = "aarch64")]
use objc::msg_send;
use objc::runtime::Object;
use objc::{sel, sel_impl};

/// `bounds` нативного NSView.
///
/// CGRect — 32 байта: на Intel macOS крупные структуры возвращаются через
/// `objc_msgSend_stret`, на Apple Silicon обычный `objc_msgSend` справляется сам.
#[cfg(target_arch = "x86_64")]
pub(crate) fn view_bounds(view: *mut Object) -> CGRect {
    unsafe extern "C" {
        fn objc_msgSend_stret(
            out: *mut CGRect,
            receiver: *mut Object,
            selector: objc::runtime::Sel,
        );
    }
    let mut frame = std::mem::MaybeUninit::<CGRect>::uninit();
    unsafe { objc_msgSend_stret(frame.as_mut_ptr(), view, sel!(bounds)) };
    unsafe { frame.assume_init() }
}

/// `bounds` нативного NSView (Apple Silicon).
#[cfg(target_arch = "aarch64")]
pub(crate) fn view_bounds(view: *mut Object) -> CGRect {
    unsafe { msg_send![view, bounds] }
}

/// Рамка контентной области: справа от сайдбара, ниже шапки, с отступами.
pub(crate) fn content_frame(view: *mut Object) -> CGRect {
    let mut frame = view_bounds(view);
    frame.origin.x = f64::from(crate::SIDEBAR_WIDTH + crate::CONTENT_MARGIN);
    frame.origin.y = f64::from(crate::CONTENT_MARGIN);
    frame.size.width =
        (frame.size.width - f64::from(crate::SIDEBAR_WIDTH + crate::CONTENT_MARGIN * 2.0)).max(0.0);
    frame.size.height = (frame.size.height
        - f64::from(crate::HEADER_HEIGHT + crate::CONTENT_MARGIN * 2.0))
    .max(0.0);
    frame
}
