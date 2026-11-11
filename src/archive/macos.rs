#![allow(unexpected_cfgs)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use block::ConcreteBlock;
use gpui::Window;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use objc_foundation::{INSString, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{ArchiveControl, PlayerEvent};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {}

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

unsafe impl objc::Encode for CMTime {
    fn encode() -> objc::Encoding {
        // SAFETY: строка — корректное описание структуры CMTime.
        unsafe { objc::Encoding::from_str("{CMTime=qiIq}") }
    }
}

unsafe extern "C" {
    fn CMTimeMakeWithSeconds(seconds: f64, preferred_timescale: i32) -> CMTime;
    #[link_name = "_dispatch_main_q"]
    static DISPATCH_MAIN_Q: Object;
}

fn main_dispatch_queue() -> *mut Object {
    (std::ptr::addr_of!(DISPATCH_MAIN_Q)).cast_mut()
}

const AV_PLAYER_ITEM_STATUS_READY: isize = 1;
const AV_PLAYER_ITEM_STATUS_FAILED: isize = 2;

fn seconds_of(time: CMTime) -> f64 {
    if time.timescale == 0 || time.value == i64::MAX {
        return 0.0;
    }
    time.value as f64 / time.timescale as f64
}

unsafe fn cmtime_of(object: *mut Object, key: &str) -> CMTime {
    let key = NSString::from_str(key);
    let value: *mut Object = msg_send![object, valueForKey: key];
    if value.is_null() {
        return CMTime {
            value: 0,
            timescale: 0,
            flags: 0,
            epoch: 0,
        };
    }
    let mut time = CMTime {
        value: 0,
        timescale: 0,
        flags: 0,
        epoch: 0,
    };
    let _: () = msg_send![value, getValue: &mut time as *mut CMTime as *mut Object];
    time
}

unsafe fn is_failed(item: *mut Object) -> bool {
    let key = NSString::from_str("status");
    let status: *mut Object = msg_send![item, valueForKey: key];
    if status.is_null() {
        return false;
    }
    let code: isize = msg_send![status, integerValue];
    code == AV_PLAYER_ITEM_STATUS_FAILED
}

unsafe fn status_code(item: *mut Object) -> isize {
    let key = NSString::from_str("status");
    let status: *mut Object = msg_send![item, valueForKey: key];
    if status.is_null() {
        return 0;
    }
    msg_send![status, integerValue]
}

unsafe fn error_description(item: *mut Object) -> Option<String> {
    let key = NSString::from_str("error");
    let error: *mut Object = msg_send![item, valueForKey: key];
    if error.is_null() {
        return None;
    }
    let description: *mut Object = msg_send![error, localizedDescription];
    if description.is_null() {
        return Some("Неизвестная ошибка воспроизведения".to_owned());
    }
    // SAFETY: указатель проверен на null выше, строка живёт в autorelease-пуле.
    Some(unsafe { (*(description as *const NSString)).as_str() }.to_owned())
}

unsafe fn rate_of(player: *mut Object) -> f32 {
    let key = NSString::from_str("rate");
    let rate: *mut Object = msg_send![player, valueForKey: key];
    if rate.is_null() {
        return 0.0;
    }
    msg_send![rate, floatValue]
}

pub struct AvPlayerControl {
    player: usize,
    item: usize,
    host_view: usize,
    observer: usize,
    speed: f32,
    alive: Arc<AtomicBool>,
}

unsafe impl Send for AvPlayerControl {}

impl ArchiveControl for AvPlayerControl {
    fn play(&mut self) {
        unsafe {
            let player = self.player as *mut Object;
            let _: () = msg_send![player, play];
            let _: () = msg_send![player, setRate: self.speed];
        }
    }

    fn pause(&mut self) {
        unsafe {
            let player = self.player as *mut Object;
            let _: () = msg_send![player, pause];
        }
    }

    fn seek(&mut self, position: f64) {
        unsafe {
            let player = self.player as *mut Object;
            let time = CMTimeMakeWithSeconds(position.max(0.0), 600);
            let _: () = msg_send![player, seekToTime: time];
        }
    }

    fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        unsafe {
            let player = self.player as *mut Object;
            let _: () = msg_send![player, setRate: speed];
        }
    }
}

impl Drop for AvPlayerControl {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        unsafe {
            let player = self.player as *mut Object;
            let _: () = msg_send![player, pause];
            if self.observer != 0 {
                let observer = self.observer as *mut Object;
                let _: () = msg_send![player, removeTimeObserver: observer];
            }
            let host_view = self.host_view as *mut Object;
            let _: () = msg_send![host_view, removeFromSuperview];
            let _: () = msg_send![host_view, release];
            let item = self.item as *mut Object;
            let _: () = msg_send![item, release];
            let _: () = msg_send![player, release];
        }
    }
}

pub fn attach(
    window: &Window,
    playlist_url: &str,
    initial_position: f64,
    events: Sender<PlayerEvent>,
) -> Result<AvPlayerControl, String> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| format!("Не удалось получить native-окно: {error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("Окно не является AppKit-окном".to_owned());
    };
    let native_view = handle.ns_view.as_ptr() as *mut Object;
    unsafe {
        let url_string = NSString::from_str(playlist_url);
        let url: *mut Object = msg_send![class!(NSURL), URLWithString: url_string];
        if url.is_null() {
            return Err("Некорректный адрес плейлиста архива".to_owned());
        }

        let frame = crate::appkit::content_frame(native_view);

        let host_view: *mut Object = msg_send![class!(NSView), alloc];
        let host_view: *mut Object = msg_send![host_view, initWithFrame: frame];
        let _: () = msg_send![host_view, setWantsLayer: true];
        let _: () = msg_send![host_view, setAutoresizingMask: 18usize];
        let _: () = msg_send![native_view, addSubview: host_view];

        let asset: *mut Object = msg_send![class!(AVURLAsset), alloc];
        let asset: *mut Object =
            msg_send![asset, initWithURL: url options: std::ptr::null::<Object>()];
        let item: *mut Object = msg_send![class!(AVPlayerItem), alloc];
        let item: *mut Object = msg_send![item, initWithAsset: asset];
        let _: () = msg_send![asset, release];
        let player: *mut Object = msg_send![class!(AVPlayer), alloc];
        let player: *mut Object = msg_send![player, initWithPlayerItem: item];
        let layer: *mut Object = msg_send![class!(AVPlayerLayer), playerLayerWithPlayer: player];
        let gravity = NSString::from_str("AVLayerVideoGravityResizeAspect");
        let _: () = msg_send![layer, setVideoGravity: gravity];
        let host_layer: *mut Object = msg_send![host_view, layer];
        let _: () = msg_send![host_layer, addSublayer: layer];
        // Кадр слоя — в координатах host_view, поэтому origin всегда (0, 0);
        // autoresizing, чтобы видео тянулось за окном.
        let mut layer_frame = frame;
        layer_frame.origin.x = 0.0;
        layer_frame.origin.y = 0.0;
        let _: () = msg_send![layer, setFrame: layer_frame];
        let _: () = msg_send![layer, setAutoresizingMask: 18usize];

        let alive = Arc::new(AtomicBool::new(true));
        let started = Arc::new(AtomicBool::new(false));
        let alive_for_block = alive.clone();
        let started_for_block = started.clone();
        let player_addr = player as usize;
        let item_addr = item as usize;
        let block = ConcreteBlock::new(move |_time: CMTime| {
            if !alive_for_block.load(Ordering::SeqCst) {
                return;
            }
            let player = player_addr as *mut Object;
            let item = item_addr as *mut Object;
            if is_failed(item) {
                let text = error_description(item)
                    .unwrap_or_else(|| "Не удалось воспроизвести архив".to_owned());
                let _ = events.send(PlayerEvent::Error(text));
                return;
            }
            if status_code(item) != AV_PLAYER_ITEM_STATUS_READY {
                return;
            }
            if !started_for_block.swap(true, Ordering::SeqCst) {
                if initial_position > 0.0 {
                    let time = CMTimeMakeWithSeconds(initial_position, 600);
                    let _: () = msg_send![player, seekToTime: time];
                }
                let _: () = msg_send![player, play];
            }
            let position = seconds_of(cmtime_of(player, "currentTime"));
            let duration = seconds_of(cmtime_of(item, "duration"));
            let paused = rate_of(player) == 0.0;
            let finished = duration > 0.0 && position >= duration - 0.3;
            let _ = events.send(PlayerEvent::State {
                position,
                duration,
                paused,
                finished,
            });
        })
        .copy();
        let interval = CMTimeMakeWithSeconds(0.5, 600);
        let observer: *mut Object = msg_send![
            player,
            addPeriodicTimeObserverForInterval: interval
            queue: main_dispatch_queue()
            usingBlock: &*block
        ];

        Ok(AvPlayerControl {
            player: player as usize,
            item: item as usize,
            host_view: host_view as usize,
            observer: observer as usize,
            speed: 1.0,
            alive,
        })
    }
}
