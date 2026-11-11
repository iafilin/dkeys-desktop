mod api;
#[cfg(target_os = "macos")]
mod appkit;
// Плеер архива сейчас реализован только на macOS; на остальных
// платформах модуль не используется — не ругаемся на dead_code.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod archive;
mod input;
mod model;
mod native_webrtc;
mod storage;

use api::{DkeysApi, normalize_phone};
#[cfg(target_os = "macos")]
use archive::ArchiveControl;
use gpui::{
    App, Application, Bounds, Context, ElementId, Entity, Focusable, FontWeight, KeyBinding,
    MouseButton, SharedString, Stateful, TitlebarOptions, Window, WindowBounds, WindowOptions,
    actions, div, img, prelude::*, px, rgb, rgba, size, white,
};
#[cfg(target_os = "macos")]
use gpui::{BorderStyle, Corners, Edges, PaintQuad, Pixels, canvas, point};
use input::TextInput;
use model::{ArchiveLink, Broadcast, Key, Session};
use native_webrtc::NativeWebView;
use storage::Storage;
use time::{OffsetDateTime, PrimitiveDateTime};

actions!(dkeys, [Quit]);

const NAVY: u32 = 0x13273f;
const BLUE: u32 = 0x3478e5;
const BACKGROUND: u32 = 0xf4f7fb;
const TEXT: u32 = 0x18263a;
const MUTED: u32 = 0x718096;
const BORDER: u32 = 0xe1e7ef;
const SIDEBAR_WIDTH: f32 = 240.0;
const HEADER_HEIGHT: f32 = 84.0;
// Используется только в нативных macOS/Windows-бэкендах (appkit, native_webrtc).
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const CONTENT_MARGIN: f32 = 28.0;
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, PartialEq)]
enum Screen {
    Phone,
    Pin,
    Keys,
    Cameras,
    CameraView(String),
    ArchivePicker(String),
    ArchiveView { code: String, date: String },
    About,
}

#[derive(Clone, PartialEq)]
enum Busy {
    RequestPin,
    VerifyPin,
    Refresh,
    Open(String),
    Camera(String),
    Archive(String),
    Logout,
}

#[derive(Clone)]
struct Status {
    text: String,
    error: bool,
}

struct DKeysApp {
    api: DkeysApi,
    storage: Storage,
    screen: Screen,
    about_return: Screen,
    session: Option<Session>,
    keys: Vec<Key>,
    cameras: Vec<Broadcast>,
    native_webrtc: Option<NativeWebView>,
    phone: Entity<TextInput>,
    pin: Entity<TextInput>,
    search: Entity<TextInput>,
    archive_date: Entity<TextInput>,
    archive_task: Option<String>,
    #[cfg(target_os = "macos")]
    archive_control: Option<archive::macos::AvPlayerControl>,
    #[cfg(target_os = "macos")]
    archive_events: Option<std::sync::mpsc::Receiver<archive::PlayerEvent>>,
    #[cfg(target_os = "macos")]
    archive_playlist: Option<archive::ArchivePlaylist>,
    #[cfg(target_os = "macos")]
    archive_position: f64,
    #[cfg(target_os = "macos")]
    archive_duration: f64,
    #[cfg(target_os = "macos")]
    archive_paused: bool,
    #[cfg(target_os = "macos")]
    archive_speed: f32,
    #[cfg(target_os = "macos")]
    archive_error: Option<String>,
    #[cfg(target_os = "macos")]
    archive_loading: bool,
    #[cfg(target_os = "macos")]
    timeline_bounds: std::sync::Arc<std::sync::Mutex<Option<Bounds<Pixels>>>>,
    busy: Option<Busy>,
    status: Option<Status>,
    confirm_key: Option<String>,
    activity: Vec<String>,
}

impl DKeysApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let storage = Storage::new().expect("configuration directory");
        let (session, keys) = storage.load();
        let screen = if session.is_some() {
            Screen::Keys
        } else {
            Screen::Phone
        };
        let phone_value = session
            .as_ref()
            .map(|session| session.phone.clone())
            .unwrap_or_default();
        let phone = cx.new(|cx| {
            TextInput::new(cx, "+7 900 123-45-67")
                .numeric(true)
                .max_len(16)
        });
        phone.update(cx, |input, cx| input.set_value(&phone_value, cx));
        Self {
            api: DkeysApi::new().expect("HTTP client"),
            storage,
            screen,
            about_return: Screen::Keys,
            session,
            keys,
            cameras: Vec::new(),
            native_webrtc: None,
            phone,
            pin: cx.new(|cx| {
                TextInput::new(cx, "Код из SMS")
                    .masked(true)
                    .numeric(true)
                    .max_len(8)
            }),
            search: cx.new(|cx| TextInput::new(cx, "Поиск по адресу или названию").max_len(80)),
            archive_date: cx.new(|cx| TextInput::new(cx, "ДД.ММ.ГГГГ ЧЧ:ММ").max_len(16)),
            archive_task: None,
            #[cfg(target_os = "macos")]
            archive_control: None,
            #[cfg(target_os = "macos")]
            archive_events: None,
            #[cfg(target_os = "macos")]
            archive_playlist: None,
            #[cfg(target_os = "macos")]
            archive_position: 0.0,
            #[cfg(target_os = "macos")]
            archive_duration: 0.0,
            #[cfg(target_os = "macos")]
            archive_paused: false,
            #[cfg(target_os = "macos")]
            archive_speed: 1.0,
            #[cfg(target_os = "macos")]
            archive_error: None,
            #[cfg(target_os = "macos")]
            archive_loading: false,
            #[cfg(target_os = "macos")]
            timeline_bounds: std::sync::Arc::new(std::sync::Mutex::new(None)),
            busy: None,
            status: None,
            confirm_key: None,
            activity: Vec::new(),
        }
    }

    fn request_pin(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let phone = normalize_phone(&self.phone.read(cx).value());
        if phone.trim_start_matches('+').len() < 11 {
            self.set_status("Введите полный номер телефона", true, cx);
            return;
        }
        self.phone
            .update(cx, |input, cx| input.set_value(&phone, cx));
        self.busy = Some(Busy::RequestPin);
        self.status = None;
        cx.notify();
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.request_pin(&phone) })
                .await;
            this.update(cx, |app, cx| {
                app.busy = None;
                match result {
                    Ok(()) => {
                        app.screen = Screen::Pin;
                        app.pin.update(cx, |input, cx| input.clear(cx));
                        app.set_status(
                            "Код отправлен. Введите его в течение нескольких минут",
                            false,
                            cx,
                        );
                    }
                    Err(error) => app.set_status(format!("{error:#}"), true, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn verify_pin(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let phone = normalize_phone(&self.phone.read(cx).value());
        let pin = self.pin.read(cx).value();
        if pin.len() < 4 {
            self.set_status("Введите код из SMS", true, cx);
            return;
        }
        self.busy = Some(Busy::VerifyPin);
        self.status = None;
        cx.notify();
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut session = api.verify_pin(&phone, &pin)?;
                    let _ = api.refresh_servers(&mut session);
                    let keys = api.load_keys(&session)?;
                    let cameras = api.load_broadcasts(&session)?;
                    anyhow::Ok((session, keys, cameras))
                })
                .await;
            this.update(cx, |app, cx| {
                app.busy = None;
                match result {
                    Ok((session, keys, cameras)) => {
                        app.session = Some(session);
                        app.keys = keys;
                        app.cameras = cameras;
                        app.screen = Screen::Keys;
                        app.pin.update(cx, |input, cx| input.clear(cx));
                        app.set_status("Вход выполнен", false, cx);
                        app.save(cx);
                    }
                    Err(error) => app.set_status(format!("{error:#}"), true, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn refresh_keys(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(mut session) = self.session.clone() else {
            return;
        };
        self.busy = Some(Busy::Refresh);
        cx.notify();
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _ = api.refresh_servers(&mut session);
                    let keys = api.load_keys(&session)?;
                    let cameras = api.load_broadcasts(&session)?;
                    anyhow::Ok((session, keys, cameras))
                })
                .await;
            this.update(cx, |app, cx| {
                app.busy = None;
                match result {
                    Ok((session, keys, cameras)) => {
                        app.session = Some(session);
                        app.keys = keys;
                        app.cameras = cameras;
                        app.set_status("Ключи и камеры обновлены", false, cx);
                        app.save(cx);
                    }
                    Err(error) => app.set_status(format!("{error:#}"), true, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn confirm_open(&mut self, code: String, cx: &mut Context<Self>) {
        if self.busy.is_none() {
            self.confirm_key = Some(code);
            cx.notify();
        }
    }

    fn open_door(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(code) = self.confirm_key.take() else {
            return;
        };
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(key) = self.keys.iter().find(|key| key.code == code).cloned() else {
            self.set_status("Ключ не найден. Обновите список", true, cx);
            return;
        };
        self.busy = Some(Busy::Open(code));
        self.status = None;
        cx.notify();
        let api = self.api.clone();
        let key_title = key.display_title().to_owned();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.open_door(&session, &key) })
                .await;
            this.update(cx, |app, cx| {
                app.busy = None;
                match result {
                    Ok(result) => {
                        let message =
                            if let Some(warning) = result.warning.filter(|text| !text.is_empty()) {
                                format!("{} · {warning}", result.message)
                            } else {
                                result.message
                            };
                        app.activity.insert(0, format!("{key_title} — {message}"));
                        app.activity.truncate(6);
                        app.set_status(message, false, cx);
                    }
                    Err(error) => app.set_status(format!("{error:#}"), true, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn show_keys(&mut self, cx: &mut Context<Self>) {
        self.stop_webrtc();
        self.screen = Screen::Keys;
        cx.notify();
    }

    fn show_cameras(&mut self, cx: &mut Context<Self>) {
        self.stop_webrtc();
        self.screen = Screen::Cameras;
        cx.notify();
        if self.cameras.is_empty() {
            self.refresh_keys(cx);
        }
    }

    fn open_camera(&mut self, code: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(camera) = self
            .cameras
            .iter()
            .find(|camera| camera.code == code)
            .cloned()
        else {
            return;
        };
        self.busy = Some(Busy::Camera(code));
        self.status = None;
        cx.notify();
        let api = self.api.clone();
        let camera_code = camera.code.clone();
        let weak_window = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.video_link(&session, &camera) })
                .await;
            this.update(cx, |app, cx| {
                app.busy = None;
                match result {
                    Ok(link) => {
                        let entity_id = cx.entity_id();
                        weak_window
                            .update(cx, |_, window, cx| {
                                app.open_webrtc_window(
                                    camera_code.clone(),
                                    link.authenticated_url(),
                                    window,
                                    entity_id,
                                    cx,
                                );
                            })
                            .ok();
                    }
                    Err(error) => app.set_status(format!("{error:#}"), true, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_webrtc_window(
        &mut self,
        code: String,
        url: String,
        window: &mut Window,
        entity_id: gpui::EntityId,
        cx: &mut App,
    ) {
        self.stop_webrtc();
        let web_url = url.replacen("rtsp://", "https://", 1);
        match native_webrtc::attach(window, &web_url) {
            Ok(view) => {
                self.native_webrtc = Some(view);
                self.screen = Screen::CameraView(code);
                self.status = None;
                cx.notify(entity_id);
            }
            Err(error) => {
                self.status = Some(Status {
                    text: error,
                    error: true,
                });
                cx.notify(entity_id);
            }
        }
    }

    fn open_archive_picker(&mut self, code: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(camera) = self.cameras.iter().find(|camera| camera.code == code) else {
            return;
        };
        if camera.archive_days().is_none() {
            self.set_status("Для этой камеры онлайн-архив недоступен", true, cx);
            return;
        }
        let now = archive_now_string();
        self.archive_date
            .update(cx, |input, cx| input.set_value(&now, cx));
        self.screen = Screen::ArchivePicker(code);
        self.status = None;
        cx.notify();
    }

    fn set_archive_day(&mut self, days: i64, cx: &mut Context<Self>) {
        let date = (archive_now() - time::Duration::days(days))
            .format(ARCHIVE_INPUT_FORMAT)
            .unwrap_or_default();
        self.archive_date
            .update(cx, |input, cx| input.set_value(&date, cx));
        self.status = None;
        cx.notify();
    }

    fn open_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Screen::ArchivePicker(code) = &self.screen else {
            return;
        };
        let code = code.clone();
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(camera) = self
            .cameras
            .iter()
            .find(|camera| camera.code == code)
            .cloned()
        else {
            return;
        };
        let Some(days) = camera.archive_days() else {
            self.set_status("Для этой камеры онлайн-архив недоступен", true, cx);
            return;
        };
        let input_date = self.archive_date.read(cx).value();
        let date_view = match archive_api_date(&input_date, days) {
            Ok(date) => date,
            Err(error) => {
                self.set_status(error, true, cx);
                return;
            }
        };
        self.busy = Some(Busy::Archive(code.clone()));
        self.status = None;
        cx.notify();
        let api = self.api.clone();
        let weak_window = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.online_archive_link(&session, &camera, &date_view) })
                .await;
            match result {
                Ok(link) => {
                    this.update(cx, |app, cx| {
                        app.busy = None;
                        cx.notify();
                    })
                    .ok();
                    weak_window
                        .update(cx, |view, window, cx| {
                            if let Ok(entity) = view.downcast::<DKeysApp>() {
                                entity.update(cx, |app, cx| {
                                    app.open_archive_view(
                                        code.clone(),
                                        input_date.clone(),
                                        link,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        })
                        .ok();
                }
                Err(error) => {
                    this.update(cx, |app, cx| {
                        app.busy = None;
                        app.set_status(format!("{error:#}"), true, cx);
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn open_archive_view(
        &mut self,
        code: String,
        date: String,
        link: ArchiveLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_webrtc();
        self.archive_task = Some(link.task.clone());
        #[cfg(target_os = "macos")]
        {
            self.archive_loading = true;
            self.archive_error = None;
            self.archive_playlist = None;
            self.archive_position = 0.0;
            self.archive_duration = 0.0;
            self.archive_paused = false;
            self.archive_speed = 1.0;
        }
        self.screen = Screen::ArchiveView {
            code: code.clone(),
            date: date.clone(),
        };
        self.status = None;
        cx.notify();

        #[cfg(not(target_os = "macos"))]
        {
            let task = link.task.clone();
            match native_webrtc::attach(window, &link.authenticated_url()) {
                Ok(view) => {
                    self.native_webrtc = Some(view);
                    cx.notify();
                }
                Err(error) => {
                    let api = self.api.clone();
                    std::thread::spawn(move || {
                        let _ = api.close_archive_session(&task);
                    });
                    self.set_status(error, true, cx);
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            let origin = link
                .link
                .split("/api/")
                .next()
                .unwrap_or("")
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .to_owned();
            let playlist_url = format!("http://{origin}/api/v1/playlist/{}", link.task);
            let initial_position = seconds_since_midnight(&date);
            let api = self.api.clone();
            let weak_window = window.window_handle();
            let fetch_url = playlist_url.clone();
            cx.spawn(async move |this, cx| {
                let playlist_text = cx
                    .background_executor()
                    .spawn(async move { api.archive_playlist_text(&fetch_url) })
                    .await;
                this.update(cx, |app, cx| {
                    app.archive_loading = false;
                    match playlist_text {
                        Ok(text) => {
                            let playlist = archive::parse_m3u8(&text);
                            if playlist.segments.is_empty() {
                                app.archive_error =
                                    Some("Сервер не вернул записи за выбранные сутки".to_owned());
                                cx.notify();
                                return;
                            }
                            app.archive_duration = playlist.duration;
                            let (tx, rx) = std::sync::mpsc::channel();
                            let url = playlist_url.clone();
                            let attached = weak_window
                                .update(cx, |_, window, _| {
                                    archive::macos::attach(window, &url, initial_position, tx)
                                })
                                .map_err(|_| "Окно закрыто".to_owned())
                                .and_then(|result| result);
                            match attached {
                                Ok(control) => {
                                    app.archive_control = Some(control);
                                    app.archive_events = Some(rx);
                                    app.archive_playlist = Some(playlist);
                                    app.spawn_archive_pump(cx);
                                    cx.notify();
                                }
                                Err(error) => {
                                    app.archive_error = Some(error);
                                    cx.notify();
                                }
                            }
                        }
                        Err(error) => {
                            app.archive_error =
                                Some(format!("Не удалось загрузить архив: {error:#}"));
                            cx.notify();
                        }
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    #[cfg(target_os = "macos")]
    fn spawn_archive_pump(&mut self, cx: &mut Context<Self>) {
        let Some(rx) = self.archive_events.take() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let mut events = Vec::new();
                while let Ok(event) = rx.try_recv() {
                    events.push(event);
                }
                let had_events = !events.is_empty();
                let alive = this
                    .update(cx, |app, cx| {
                        if had_events {
                            for event in events {
                                app.apply_archive_event(event);
                            }
                            cx.notify();
                        }
                        app.archive_control.is_some()
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    #[cfg(target_os = "macos")]
    fn apply_archive_event(&mut self, event: archive::PlayerEvent) {
        match event {
            archive::PlayerEvent::State {
                position,
                duration,
                paused,
                finished,
            } => {
                self.archive_position = position;
                if duration > 0.0 {
                    self.archive_duration = duration;
                }
                self.archive_paused = paused || finished;
            }
            archive::PlayerEvent::Error(text) => {
                self.archive_error = Some(text);
            }
        }
    }

    fn stop_webrtc(&mut self) {
        if let Some(view) = self.native_webrtc.take() {
            native_webrtc::detach(view);
        }
        #[cfg(target_os = "macos")]
        {
            self.archive_control = None;
            self.archive_events = None;
            self.archive_playlist = None;
            self.archive_error = None;
            self.archive_loading = false;
        }
        if let Some(task) = self.archive_task.take() {
            let api = self.api.clone();
            std::thread::spawn(move || {
                let _ = api.close_archive_session(&task);
            });
        }
    }

    fn close_camera(&mut self, cx: &mut Context<Self>) {
        self.stop_webrtc();
        self.screen = Screen::Cameras;
        self.status = None;
        cx.notify();
    }

    fn logout(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.busy = Some(Busy::Logout);
        cx.notify();
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.logout(&session) })
                .await;
            this.update(cx, |app, cx| {
                app.busy = None;
                app.session = None;
                app.keys.clear();
                app.cameras.clear();
                app.stop_webrtc();
                app.activity.clear();
                app.screen = Screen::Phone;
                let _ = app.storage.clear();
                match result {
                    Ok(()) => app.status = None,
                    Err(error) => app.set_status(
                        format!("Локальный выход выполнен. Сервер: {error:#}"),
                        true,
                        cx,
                    ),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn back_to_phone(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_none() {
            self.screen = Screen::Phone;
            self.status = None;
            self.pin.update(cx, |input, cx| input.clear(cx));
            cx.notify();
        }
    }

    fn show_about(&mut self, cx: &mut Context<Self>) {
        self.about_return = self.screen.clone();
        self.stop_webrtc();
        self.screen = Screen::About;
        self.status = None;
        cx.notify();
    }

    fn close_about(&mut self, cx: &mut Context<Self>) {
        self.screen = if matches!(
            self.about_return,
            Screen::CameraView(_) | Screen::ArchivePicker(_) | Screen::ArchiveView { .. }
        ) {
            Screen::Cameras
        } else {
            self.about_return.clone()
        };
        cx.notify();
    }

    fn open_developer(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = open::that("https://vk.ru/iafilin") {
            self.set_status(
                format!("Не удалось открыть страницу разработчика: {error}"),
                true,
                cx,
            );
        }
    }

    fn open_email(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = open::that("mailto:iafilin@vk.com") {
            self.set_status(format!("Не удалось открыть почту: {error}"), true, cx);
        }
    }

    fn open_repository(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = open::that("https://github.com/iafilin/dkeys-desktop") {
            self.set_status(format!("Не удалось открыть репозиторий: {error}"), true, cx);
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.storage.save(self.session.as_ref(), &self.keys) {
            self.set_status(error.to_string(), true, cx);
        }
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.status = Some(Status {
            text: text.into(),
            error,
        });
        cx.notify();
    }

    fn busy_label(&self) -> Option<&str> {
        match self.busy.as_ref()? {
            Busy::RequestPin => Some("Отправляем код…"),
            Busy::VerifyPin => Some("Проверяем код…"),
            Busy::Refresh => Some("Обновляем данные…"),
            Busy::Open(_) => Some("Отправляем команду…"),
            Busy::Camera(_) => Some("Подключаем камеру…"),
            Busy::Archive(_) => Some("Получаем онлайн-архив…"),
            Busy::Logout => Some("Выходим…"),
        }
    }

    fn render_auth(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_pin = self.screen == Screen::Pin;
        let busy = self.busy.is_some();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(BACKGROUND))
            .child(
                div()
                    .w(px(452.))
                    .flex()
                    .flex_col()
                    .gap_5()
                    .p_8()
                    .bg(white())
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .shadow_lg()
                    .child(brand(true))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child(if is_pin {
                                        "Подтверждение входа"
                                    } else {
                                        "Ваши ключи — на компьютере"
                                    }),
                            )
                            .child(div().text_sm().text_color(rgb(MUTED)).child(if is_pin {
                                format!("Отправили SMS на {}", self.phone.read(cx).value())
                            } else {
                                "Войдите с номером, привязанным к домофону D-KEYS".to_owned()
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(TEXT))
                                    .child(if is_pin {
                                        "Код из SMS"
                                    } else {
                                        "Номер телефона"
                                    }),
                            )
                            .child(input_container(if is_pin {
                                self.pin.clone()
                            } else {
                                self.phone.clone()
                            })),
                    )
                    .children(self.status.as_ref().map(render_status))
                    .child(
                        action_button(
                            "auth-submit",
                            if is_pin {
                                "Войти"
                            } else {
                                "Получить код"
                            },
                            true,
                        )
                        .when(busy, |button| button.opacity(0.55))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|app, _, _, cx| {
                                if app.screen == Screen::Pin {
                                    app.verify_pin(cx);
                                } else {
                                    app.request_pin(cx);
                                }
                            }),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(text_button("about-auth", "О приложении").on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.show_about(cx)),
                            ))
                            .when(is_pin, |row| {
                                row.child(text_button("auth-back", "Изменить номер").on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|app, _, _, cx| app.back_to_phone(cx)),
                                ))
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("Доступен только для зарегистрированных пользователей D-KEYS"),
                    ),
            )
    }

    fn render_keys(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value();
        let keys: Vec<&Key> = self.keys.iter().filter(|key| key.matches(&query)).collect();
        let keys_empty = keys.is_empty();
        let mut key_cards = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            key_cards.push(self.render_key_card(index, key, cx).into_any_element());
        }
        let phone = self
            .session
            .as_ref()
            .map(|session| session.phone.clone())
            .unwrap_or_default();
        let busy = self.busy.is_some();
        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(HEADER_HEIGHT))
                            .w_full()
                            .px_7()
                            .flex()
                            .items_center()
                            .justify_between()
                            .bg(white())
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(TEXT))
                                            .child("Мои ключи"),
                                    )
                                    .child(div().text_sm().text_color(rgb(MUTED)).child(phone)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .items_center()
                                    .children(self.busy_label().map(|label| {
                                        div()
                                            .text_sm()
                                            .text_color(rgb(MUTED))
                                            .child(label.to_owned())
                                    }))
                                    .child(
                                        action_button("refresh", "Обновить", false)
                                            .when(busy, |button| button.opacity(0.55))
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|app, _, _, cx| app.refresh_keys(cx)),
                                            ),
                                    )
                                    .child(text_button("logout", "Выйти").on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|app, _, _, cx| app.logout(cx)),
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .px_7()
                            .py_5()
                            .flex()
                            .gap_4()
                            .items_center()
                            .child(
                                div()
                                    .w(px(390.))
                                    .child(input_container(self.search.clone())),
                            )
                            .child(div().text_sm().text_color(rgb(MUTED)).child(format_count(
                                keys.len(),
                                "ключ",
                                "ключа",
                                "ключей",
                            ))),
                    )
                    .children(
                        self.status
                            .as_ref()
                            .map(|status| div().px_7().pb_4().child(render_status(status))),
                    )
                    .child(
                        div()
                            .id("keys-scroll")
                            .flex_1()
                            .overflow_scroll()
                            .px_7()
                            .pb_7()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_wrap()
                                    .gap_4()
                                    .children(key_cards)
                                    .when(keys_empty, |container| {
                                        container.child(
                                            div()
                                                .w_full()
                                                .py_10()
                                                .flex()
                                                .flex_col()
                                                .items_center()
                                                .gap_3()
                                                .child(div().text_size(px(42.)).child("⌂"))
                                                .child(
                                                    div()
                                                        .text_lg()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(rgb(TEXT))
                                                        .child(if query.trim().is_empty() {
                                                            "Пока нет доступных ключей"
                                                        } else {
                                                            "Ничего не найдено"
                                                        }),
                                                )
                                                .child(
                                                    div().text_sm().text_color(rgb(MUTED)).child(
                                                        "Проверьте поиск или обновите список",
                                                    ),
                                                ),
                                        )
                                    }),
                            ),
                    )
                    .when(!self.activity.is_empty(), |content| {
                        content.child(
                            div()
                                .px_7()
                                .py_4()
                                .bg(white())
                                .border_t_1()
                                .border_color(rgb(BORDER))
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(MUTED))
                                        .child("ПОСЛЕДНИЕ ДЕЙСТВИЯ"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(TEXT))
                                        .child(self.activity[0].clone()),
                                ),
                        )
                    }),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .bg(rgb(NAVY))
            .flex()
            .flex_col()
            .p_5()
            .child(brand(false))
            .child(
                div()
                    .mt_8()
                    .mb_3()
                    .px_3()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(0xffffff66))
                    .child("РАЗДЕЛЫ"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .id("nav-keys")
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .rounded_lg()
                            .text_color(if self.screen == Screen::Keys {
                                rgb(0xffffff)
                            } else {
                                rgba(0xffffffaa)
                            })
                            .when(self.screen == Screen::Keys, |element| {
                                element.bg(rgba(0xffffff20))
                            })
                            .hover(|style| style.bg(rgba(0xffffff12)).text_color(white()))
                            .child("▣")
                            .child("Мои ключи")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.show_keys(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("nav-cameras")
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .rounded_lg()
                            .text_color(
                                if matches!(
                                    self.screen,
                                    Screen::Cameras
                                        | Screen::CameraView(_)
                                        | Screen::ArchivePicker(_)
                                        | Screen::ArchiveView { .. }
                                ) {
                                    rgb(0xffffff)
                                } else {
                                    rgba(0xffffffaa)
                                },
                            )
                            .when(
                                matches!(
                                    self.screen,
                                    Screen::Cameras
                                        | Screen::CameraView(_)
                                        | Screen::ArchivePicker(_)
                                        | Screen::ArchiveView { .. }
                                ),
                                |element| element.bg(rgba(0xffffff20)),
                            )
                            .hover(|style| style.bg(rgba(0xffffff12)).text_color(white()))
                            .child("◉")
                            .child("Камеры")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.show_cameras(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("about-side")
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .rounded_lg()
                            .when(self.screen == Screen::About, |element| {
                                element.bg(rgba(0xffffff20))
                            })
                            .text_color(rgba(0xffffffaa))
                            .hover(|style| style.bg(rgba(0xffffff12)).text_color(white()))
                            .child("ⓘ")
                            .child("О приложении")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.show_about(cx)),
                            ),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_lg()
                    .bg(rgba(0xffffff0b))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgba(0xffffffcc))
                            .child(format!("D-KEYS Desktop {VERSION}")),
                    )
                    .child(
                        div()
                            .id("developer-link")
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgba(0xffffff99))
                            .hover(|style| style.text_color(white()))
                            .child("Игорь Филин · vk.ru/iafilin")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.open_developer(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("email-link")
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgba(0xffffff77))
                            .hover(|style| style.text_color(white()))
                            .child("iafilin@vk.com")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.open_email(cx)),
                            ),
                    ),
            )
    }

    fn render_about(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(HEADER_HEIGHT))
                            .px_7()
                            .flex()
                            .items_center()
                            .gap_4()
                            .bg(white())
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                text_button("close-about", "← Назад")
                                    .on_mouse_up(MouseButton::Left, cx.listener(|app, _, _, cx| app.close_about(cx))),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child("О приложении"),
                            ),
                    )
                    .child(
                        div()
                            .id("about-scroll")
                            .flex_1()
                            .overflow_scroll()
                            .p_7()
                            .child(
                                div()
                                    .max_w(px(760.))
                                    .flex()
                                    .flex_col()
                                    .gap_5()
                                    .child(
                                        div()
                                            .p_7()
                                            .flex()
                                            .flex_col()
                                            .gap_3()
                                            .rounded_xl()
                                            .bg(rgb(NAVY))
                                            .child(
                                                div()
                                                    .text_2xl()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(white())
                                                    .child("D-KEYS Desktop"),
                                            )
                                            .child(
                                                div()
                                                    .text_lg()
                                                    .text_color(rgba(0xffffffcc))
                                                    .child("Домофон и камеры — в одном desktop-приложении"),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(rgba(0xffffff88))
                                                    .child(format!("Версия {VERSION} · неофициальный порт Android-приложения")),
                                            ),
                                    )
                                    .child(
                                        about_section(
                                            "Разработчик",
                                            "Игорь Филин. Desktop-версия разработана независимо от команды D-KEYS. По вопросам работы этой сборки обращайтесь напрямую:",
                                        )
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_3()
                                            .child(
                                                action_button("about-developer", "Игорь Филин · vk.ru", false)
                                                    .on_mouse_up(MouseButton::Left, cx.listener(|app, _, _, cx| app.open_developer(cx))),
                                            )
                                            .child(
                                                action_button("about-email", "iafilin@vk.com", false)
                                                    .on_mouse_up(MouseButton::Left, cx.listener(|app, _, _, cx| app.open_email(cx))),
                                            )
                                            .child(
                                                action_button("about-repository", "Исходный код · GitHub", false)
                                                    .on_mouse_up(MouseButton::Left, cx.listener(|app, _, _, cx| app.open_repository(cx))),
                                            ),
                                    )
                                    .child(
                                        about_section(
                                            "Отказ от ответственности",
                                            "Это неофициальный порт оригинального Android-приложения D-KEYS. Разработчики D-KEYS не участвуют в создании этой версии, не несут ответственности за её работу, безопасность, совместимость и последствия использования, а также не оказывают ей техническую поддержку. Название D-KEYS и связанные с ним права принадлежат их правообладателям.",
                                        )
                                    )
                                    .child(
                                        about_section(
                                            "Данные и доступ",
                                            "Телефон, токен сессии и кэш ключей хранятся локально в session.json; на macOS файл получает права 0600. PIN-код не сохраняется. Используйте приложение только для оборудования, к которому у вас есть законный доступ. Открытие дверей выполняется после дополнительного подтверждения.",
                                        )
                                    )
                                    .child(
                                        about_section(
                                            "Возможности и ограничения",
                                            "Вход по SMS, список ключей, удалённое открытие, живые камеры по WebRTC и онлайн-архив с выбором даты внутри приложения. На macOS используется WebKit, на Windows — WebView2. Внешний браузер, RTSP и FFmpeg для просмотра не требуются. Доступность оборудования и архивных записей зависит от серверов D-KEYS и вашей учётной записи.",
                                        )
                                    )
                                    .children(self.status.as_ref().map(render_status)),
                            ),
                    ),
            )
    }

    fn render_cameras(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy.is_some();
        let mut cards = Vec::new();
        for (index, camera) in self.cameras.iter().enumerate() {
            cards.push(
                self.render_camera_card(index, camera, cx)
                    .into_any_element(),
            );
        }
        let empty = cards.is_empty();
        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(HEADER_HEIGHT))
                            .w_full()
                            .px_7()
                            .flex()
                            .items_center()
                            .justify_between()
                            .bg(white())
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(TEXT))
                                            .child("Камеры"),
                                    )
                                    .child(div().text_sm().text_color(rgb(MUTED)).child(
                                        format_count(
                                            self.cameras.len(),
                                            "видеопоток",
                                            "видеопотока",
                                            "видеопотоков",
                                        ),
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .items_center()
                                    .children(self.busy_label().map(|label| {
                                        div()
                                            .text_sm()
                                            .text_color(rgb(MUTED))
                                            .child(label.to_owned())
                                    }))
                                    .child(
                                        action_button("refresh-cameras", "Обновить", false)
                                            .when(busy, |button| button.opacity(0.55))
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|app, _, _, cx| app.refresh_keys(cx)),
                                            ),
                                    ),
                            ),
                    )
                    .children(
                        self.status
                            .as_ref()
                            .map(|status| div().px_7().py_4().child(render_status(status))),
                    )
                    .child(
                        div()
                            .id("cameras-scroll")
                            .flex_1()
                            .overflow_scroll()
                            .p_7()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_wrap()
                                    .gap_4()
                                    .children(cards)
                                    .when(empty, |container| {
                                        container.child(
                                        div()
                                            .w_full()
                                            .py_10()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap_3()
                                            .child(div().text_size(px(42.)).child("◉"))
                                            .child(
                                                div()
                                                    .text_lg()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(TEXT))
                                                    .child("Нет доступных камер"),
                                            )
                                            .child(div().text_sm().text_color(rgb(MUTED)).child(
                                                "Нажмите «Обновить», чтобы запросить список снова",
                                            )),
                                    )
                                    }),
                            ),
                    ),
            )
    }

    fn render_camera_card(
        &self,
        index: usize,
        camera: &Broadcast,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let code = camera.code.clone();
        let archive_code = camera.code.clone();
        let connecting = matches!(self.busy.as_ref(), Some(Busy::Camera(open_code)) if open_code == &camera.code);
        let archive_connecting = matches!(self.busy.as_ref(), Some(Busy::Archive(open_code)) if open_code == &camera.code);
        div()
            .w(px(402.))
            .min_h(px(180.))
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .rounded_xl()
            .bg(white())
            .border_1()
            .border_color(rgb(BORDER))
            .shadow_sm()
            .hover(|style| style.border_color(rgb(0xabc4ea)).shadow_md())
            .child(
                div()
                    .flex()
                    .gap_4()
                    .items_start()
                    .child(
                        div()
                            .w(px(48.))
                            .h(px(48.))
                            .flex_shrink_0()
                            .rounded_md()
                            .bg(rgb(key_color(&camera.color)))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xl()
                            .text_color(white())
                            .child("◉"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child(ellipsize(camera.display_title(), 28)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child(ellipsize(camera.display_address(), 44)),
                            ),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(
                                if camera.online > 0 {
                                    rgb(0x2c9b69)
                                } else {
                                    rgb(0xa42b35)
                                },
                            ))
                            .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                                "{} · {}",
                                if camera.online > 0 {
                                    "Онлайн"
                                } else {
                                    "Офлайн"
                                },
                                camera.protocol
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .when(camera.archive_days().is_some(), |buttons| {
                                buttons.child(
                                    action_button(
                                        ElementId::from(("archive", index)),
                                        if archive_connecting {
                                            "Загрузка…"
                                        } else {
                                            "Архив"
                                        },
                                        false,
                                    )
                                    .when(archive_connecting, |button| button.opacity(0.55))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |app, _, _, cx| {
                                            app.open_archive_picker(archive_code.clone(), cx);
                                        }),
                                    ),
                                )
                            })
                            .child(
                                action_button(
                                    ElementId::from(("watch", index)),
                                    if connecting {
                                        "Подключаем…"
                                    } else {
                                        "Смотреть"
                                    },
                                    true,
                                )
                                .when(connecting, |button| button.opacity(0.55))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(move |app, _, window, cx| {
                                        app.open_camera(code.clone(), window, cx);
                                    }),
                                ),
                            ),
                    ),
            )
    }

    fn render_archive_picker(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, days) = if let Screen::ArchivePicker(code) = &self.screen {
            self.cameras
                .iter()
                .find(|camera| &camera.code == code)
                .map(|camera| {
                    (
                        camera.display_title().to_owned(),
                        camera.archive_days().unwrap_or_default(),
                    )
                })
                .unwrap_or_else(|| ("Камера".to_owned(), 0))
        } else {
            ("Камера".to_owned(), 0)
        };
        let loading = matches!(self.busy.as_ref(), Some(Busy::Archive(_)));
        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(HEADER_HEIGHT))
                            .px_7()
                            .flex()
                            .items_center()
                            .gap_4()
                            .bg(white())
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(text_button("close-archive-picker", "← Назад").on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.close_camera(cx)),
                            ))
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child("Онлайн-архив"),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .p_7()
                            .child(
                                div()
                                    .w(px(560.))
                                    .p_7()
                                    .flex()
                                    .flex_col()
                                    .gap_5()
                                    .rounded_xl()
                                    .bg(white())
                                    .border_1()
                                    .border_color(rgb(BORDER))
                                    .shadow_sm()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_2xl()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(rgb(TEXT))
                                                    .child("Выберите дату и время"),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(rgb(MUTED))
                                                    .child(title),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(TEXT))
                                                    .child("Начало записи"),
                                            )
                                            .child(input_container(self.archive_date.clone()))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(MUTED))
                                                    .child("Формат: ДД.ММ.ГГГГ ЧЧ:ММ"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_3()
                                            .child(
                                                action_button("archive-today", "Сегодня", false)
                                                    .on_mouse_up(
                                                        MouseButton::Left,
                                                        cx.listener(|app, _, _, cx| app.set_archive_day(0, cx)),
                                                    ),
                                            )
                                            .child(
                                                action_button("archive-yesterday", "Вчера", false)
                                                    .on_mouse_up(
                                                        MouseButton::Left,
                                                        cx.listener(|app, _, _, cx| app.set_archive_day(1, cx)),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .p_4()
                                            .rounded_lg()
                                            .bg(rgb(0xedf3fb))
                                            .text_sm()
                                            .text_color(rgb(NAVY))
                                            .child(format!(
                                                "Доступны записи за последние {days} дн. Время выбирается по местному часовому поясу."
                                            )),
                                    )
                                    .children(self.status.as_ref().map(render_status))
                                    .child(
                                        action_button(
                                            "open-archive",
                                            if loading { "Получаем ссылку…" } else { "Смотреть архив" },
                                            true,
                                        )
                                        .when(loading, |button| button.opacity(0.55))
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|app, _, window, cx| app.open_archive(window, cx)),
                                        ),
                                    ),
                            ),
                    ),
            )
    }

    fn render_camera_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, protocol) = match &self.screen {
            Screen::CameraView(code) => self
                .cameras
                .iter()
                .find(|camera| &camera.code == code)
                .map(|camera| (camera.display_title().to_owned(), camera.protocol.clone()))
                .unwrap_or_else(|| ("Камера".to_owned(), String::new())),
            Screen::ArchiveView { code, date } => self
                .cameras
                .iter()
                .find(|camera| &camera.code == code)
                .map(|camera| {
                    (
                        format!("{} · {date}", camera.display_title()),
                        "Онлайн-архив".to_owned(),
                    )
                })
                .unwrap_or_else(|| (format!("Онлайн-архив · {date}"), String::new())),
            _ => ("Камера".to_owned(), String::new()),
        };
        let frame: Option<std::sync::Arc<gpui::RenderImage>> = None;
        let has_frame = frame.is_some();
        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(HEADER_HEIGHT))
                            .px_7()
                            .flex()
                            .items_center()
                            .gap_4()
                            .bg(white())
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(text_button("close-camera", "← Назад").on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|app, _, _, cx| app.close_camera(cx)),
                            ))
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child(title),
                            )
                            .child(div().flex_1())
                            .child(div().text_sm().text_color(rgb(MUTED)).child(protocol)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .m_7()
                            .rounded_lg()
                            .bg(rgb(0x050a12))
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(frame.map(|frame| img(frame).size_full().into_any_element()))
                            .when(!has_frame, |container| {
                                container.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .text_size(px(42.))
                                                .text_color(rgba(0xffffff88))
                                                .child("◉"),
                                        )
                                        .child(
                                            div()
                                                .text_lg()
                                                .text_color(rgba(0xffffffbb))
                                                .child("Подключаемся к видеопотоку…"),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgba(0xffffff77))
                                                .child("Первый кадр появится автоматически"),
                                        ),
                                )
                            }),
                    )
                    .children(
                        self.status
                            .as_ref()
                            .map(|status| div().px_7().pb_5().child(render_status(status))),
                    ),
            )
    }

    #[cfg(target_os = "macos")]
    fn render_archive_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match &self.screen {
            Screen::ArchiveView { code, date } => self
                .cameras
                .iter()
                .find(|camera| &camera.code == code)
                .map(|camera| format!("{} · {date}", camera.display_title()))
                .unwrap_or_else(|| format!("Онлайн-архив · {date}")),
            _ => "Онлайн-архив".to_owned(),
        };
        let loading = self.archive_loading;
        let error = self.archive_error.clone();
        let has_player = self.archive_control.is_some();
        let paused = self.archive_paused;
        let speed = self.archive_speed;
        let position = self.archive_position;
        let duration = self.archive_duration;

        let mut content = div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(HEADER_HEIGHT))
                    .px_7()
                    .flex()
                    .items_center()
                    .gap_4()
                    .bg(white())
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(text_button("close-camera", "← Назад").on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|app, _, _, cx| app.close_camera(cx)),
                    ))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .child(title),
                    )
                    .child(div().flex_1())
                    .child(div().text_sm().text_color(rgb(MUTED)).child("Онлайн-архив")),
            )
            .child(
                div()
                    .flex_1()
                    .m_7()
                    .rounded_lg()
                    .bg(rgb(0x050a12))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(loading, |area| {
                        area.child(
                            div()
                                .text_lg()
                                .text_color(rgba(0xffffffbb))
                                .child("Загружаем архив…"),
                        )
                    })
                    .when(!loading && error.is_none() && !has_player, |area| {
                        area.child(
                            div()
                                .text_lg()
                                .text_color(rgba(0xffffffbb))
                                .child("Подключаемся к видео…"),
                        )
                    })
                    .when_some(error.clone(), |area, text| {
                        area.child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_4()
                                .child(
                                    div()
                                        .max_w(px(480.))
                                        .text_center()
                                        .text_color(rgba(0xffb4b4ff))
                                        .child(text),
                                )
                                .child(text_button("retry-archive", "Повторить").on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|app, _, window, cx| {
                                        let code = match &app.screen {
                                            Screen::ArchiveView { code, .. } => code.clone(),
                                            _ => return,
                                        };
                                        app.stop_webrtc();
                                        app.screen = Screen::ArchivePicker(code);
                                        app.open_archive(window, cx);
                                    }),
                                )),
                        )
                    }),
            );

        if has_player {
            let controls = div()
                .px_7()
                .pb_3()
                .flex()
                .items_center()
                .gap_3()
                .child(player_button(if paused { "⏵" } else { "⏸" }).on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|app, _, _, cx| {
                        if let Some(control) = app.archive_control.as_mut() {
                            if app.archive_paused {
                                control.play();
                            } else {
                                control.pause();
                            }
                        }
                        cx.notify();
                    }),
                ))
                .child(player_pill("−15 с").on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|app, _, _, cx| {
                        let target = (app.archive_position - 15.0).max(0.0);
                        if let Some(control) = app.archive_control.as_mut() {
                            control.seek(target);
                            app.archive_position = target;
                        }
                        cx.notify();
                    }),
                ))
                .child(player_pill("+15 с").on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|app, _, _, cx| {
                        let target =
                            (app.archive_position + 15.0).min(app.archive_duration.max(0.0));
                        if let Some(control) = app.archive_control.as_mut() {
                            control.seek(target);
                            app.archive_position = target;
                        }
                        cx.notify();
                    }),
                ))
                .child(player_pill(&format!("×{speed}")).on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|app, _, _, cx| {
                        const SPEEDS: [f32; 4] = [1.0, 1.5, 2.0, 4.0];
                        let next = SPEEDS
                            .iter()
                            .find(|&&value| value > app.archive_speed)
                            .copied()
                            .unwrap_or(SPEEDS[0]);
                        app.archive_speed = next;
                        if let Some(control) = app.archive_control.as_mut() {
                            control.set_speed(next);
                        }
                        cx.notify();
                    }),
                ))
                .child(div().flex_1())
                .child(div().text_sm().text_color(rgb(TEXT)).child(format!(
                    "{} / {}",
                    archive::format_time(position),
                    archive::format_time(duration)
                )));
            content = content
                .child(controls)
                .child(self.render_archive_timeline(cx));
        }

        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .child(self.render_sidebar(cx))
            .child(content)
    }

    #[cfg(target_os = "macos")]
    fn render_archive_timeline(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let playlist = self.archive_playlist.clone();
        let position = self.archive_position;
        let duration = self.archive_duration;
        let bounds_cell = self.timeline_bounds.clone();
        div()
            .mx_7()
            .mb_5()
            .h(px(48.))
            .rounded_md()
            .bg(rgb(0x0d1420))
            .overflow_hidden()
            .child(canvas(
                move |bounds, _, _| {
                    *bounds_cell.lock().unwrap() = Some(bounds);
                    (playlist, position, duration)
                },
                |bounds, (playlist, position, duration), window, _| {
                    let Some(playlist) = playlist else {
                        return;
                    };
                    if playlist.segments.is_empty() || duration <= 0.0 {
                        return;
                    }
                    let bar_height = px(8.0);
                    let bar_y = bounds.center().y - bar_height / 2.0;
                    for segment in &playlist.segments {
                        let x =
                            bounds.left() + bounds.size.width * (segment.start / duration) as f32;
                        let width =
                            (bounds.size.width * (segment.duration / duration) as f32).max(px(1.0));
                        let color = match segment.color {
                            archive::SegmentColor::Recording => rgb(0xe5484d),
                            archive::SegmentColor::Placeholder => rgb(0x2b3444),
                            archive::SegmentColor::Unknown => rgb(0x3a4356),
                        };
                        window.paint_quad(PaintQuad {
                            bounds: Bounds::new(point(x, bar_y), size(width, bar_height)),
                            corner_radii: Corners::default(),
                            background: color.into(),
                            border_widths: Edges::default(),
                            border_color: rgb(0x000000).into(),
                            border_style: BorderStyle::default(),
                        });
                    }
                    let thumb_x = bounds.left()
                        + bounds.size.width * ((position / duration).clamp(0.0, 1.0) as f32);
                    window.paint_quad(PaintQuad {
                        bounds: Bounds::new(
                            point(thumb_x - px(7.0), bounds.center().y - px(7.0)),
                            size(px(14.0), px(14.0)),
                        ),
                        corner_radii: Corners::all(px(7.0)),
                        background: white().into(),
                        border_widths: Edges::default(),
                        border_color: rgb(0x000000).into(),
                        border_style: BorderStyle::default(),
                    });
                },
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|app, event: &gpui::MouseDownEvent, _, cx| {
                    if let Some(bounds) = *app.timeline_bounds.lock().unwrap()
                        && bounds.size.width > px(0.0)
                        && app.archive_duration > 0.0
                    {
                        let fraction = ((event.position.x - bounds.left()) / bounds.size.width)
                            .clamp(0.0, 1.0);
                        let target = f64::from(fraction) * app.archive_duration;
                        if let Some(control) = app.archive_control.as_mut() {
                            control.seek(target);
                            app.archive_position = target;
                        }
                        cx.notify();
                    }
                }),
            )
    }

    fn render_key_card(&self, index: usize, key: &Key, cx: &mut Context<Self>) -> impl IntoElement {
        let code = key.code.clone();
        let opening =
            matches!(self.busy.as_ref(), Some(Busy::Open(open_code)) if open_code == &key.code);
        div()
            .w(px(402.))
            .min_h(px(190.))
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .rounded_xl()
            .bg(white())
            .border_1()
            .border_color(rgb(BORDER))
            .shadow_sm()
            .hover(|style| style.border_color(rgb(0xabc4ea)).shadow_md())
            .child(
                div()
                    .flex()
                    .gap_4()
                    .items_start()
                    .child(
                        div()
                            .w(px(48.))
                            .h(px(48.))
                            .flex_shrink_0()
                            .rounded_md()
                            .bg(rgb(key_color(&key.color)))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white())
                            .child(
                                key.display_title()
                                    .chars()
                                    .next()
                                    .unwrap_or('D')
                                    .to_uppercase()
                                    .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child(ellipsize(key.display_title(), 28)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child(ellipsize(key.display_address(), 44)),
                            ),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("Код {}", key.code)),
                    )
                    .child(
                        action_button(
                            ElementId::from(("open", index)),
                            if opening {
                                "Открываем…"
                            } else {
                                "Открыть"
                            },
                            true,
                        )
                        .when(opening, |button| button.opacity(0.55))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |app, _, _, cx| {
                                app.confirm_open(code.clone(), cx);
                            }),
                        ),
                    ),
            )
    }

    fn render_confirmation(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let code = self.confirm_key.as_ref()?;
        let key = self.keys.iter().find(|key| &key.code == code)?.clone();
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(0x07182dbb))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(430.))
                        .p_7()
                        .flex()
                        .flex_col()
                        .gap_5()
                        .bg(white())
                        .rounded_xl()
                        .shadow_lg()
                        .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).text_color(rgb(TEXT)).child("Открыть дверь?"))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(rgb(TEXT)).child(ellipsize(key.display_title(), 28)))
                                .child(div().text_sm().text_color(rgb(MUTED)).child(ellipsize(key.display_address(), 44))),
                        )
                        .child(
                            div()
                                .p_3()
                                .rounded_md()
                                .bg(rgb(0xfff6df))
                                .text_sm()
                                .text_color(rgb(0x795b16))
                                .child("Команда будет отправлена на домофон через сервер D-KEYS. Убедитесь, что выбран правильный адрес."),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_3()
                                .child(
                                    action_button("cancel-open", "Отмена", false)
                                        .on_mouse_up(MouseButton::Left, cx.listener(|app, _, _, cx| {
                                            app.confirm_key = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    action_button("confirm-open", "Да, открыть", true)
                                        .on_mouse_up(MouseButton::Left, cx.listener(|app, _, _, cx| app.open_door(cx))),
                                ),
                        ),
                ),
        )
    }
}

impl Drop for DKeysApp {
    fn drop(&mut self) {
        self.stop_webrtc();
    }
}

impl Render for DKeysApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(view) = self.native_webrtc.as_ref() {
            native_webrtc::resize(view, window);
        }
        div()
            .relative()
            .size_full()
            .text_color(rgb(TEXT))
            .child(match &self.screen {
                Screen::Phone | Screen::Pin => self.render_auth(cx).into_any_element(),
                Screen::Keys => self.render_keys(cx).into_any_element(),
                Screen::Cameras => self.render_cameras(cx).into_any_element(),
                Screen::CameraView(_) => self.render_camera_view(cx).into_any_element(),
                Screen::ArchivePicker(_) => self.render_archive_picker(cx).into_any_element(),
                #[cfg(target_os = "macos")]
                Screen::ArchiveView { .. } => self.render_archive_view(cx).into_any_element(),
                #[cfg(not(target_os = "macos"))]
                Screen::ArchiveView { .. } => self.render_camera_view(cx).into_any_element(),
                Screen::About => self.render_about(cx).into_any_element(),
            })
            .children(
                self.render_confirmation(cx)
                    .map(|modal| modal.into_any_element()),
            )
    }
}

fn brand(dark: bool) -> impl IntoElement {
    let foreground = if dark { NAVY } else { 0xffffff };
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(44.))
                .h(px(44.))
                .rounded_lg()
                .bg(if dark { rgb(BLUE) } else { rgb(0xffffff) })
                .flex()
                .items_center()
                .justify_center()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(if dark { rgb(0xffffff) } else { rgb(NAVY) })
                .child("D"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(foreground))
                        .child("D-KEYS"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if dark { rgb(MUTED) } else { rgba(0xffffff88) })
                        .child("DESKTOP"),
                ),
        )
}

fn input_container(input: Entity<TextInput>) -> impl IntoElement {
    div()
        .h(px(52.))
        .w_full()
        .px_4()
        .flex()
        .items_center()
        .rounded_lg()
        .bg(white())
        .border_1()
        .border_color(rgb(BORDER))
        .child(input)
}

fn action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
) -> Stateful<gpui::Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .cursor_pointer()
        .h(px(44.))
        .px_5()
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .when(primary, |button| {
            button
                .bg(rgb(BLUE))
                .text_color(white())
                .shadow_sm()
                .hover(|style| style.bg(rgb(0x2865c4)))
        })
        .when(!primary, |button| {
            button
                .bg(white())
                .text_color(rgb(NAVY))
                .border_1()
                .border_color(rgb(BORDER))
                .hover(|style| style.bg(rgb(0xedf3fb)).border_color(rgb(0xc4d4e8)))
        })
        .child(label)
}

fn text_button(id: &'static str, label: &'static str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .px_3()
        .py_2()
        .rounded_lg()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(BLUE))
        .hover(|style| style.bg(rgb(0xeaf2ff)))
        .child(label)
}

#[cfg(target_os = "macos")]
fn player_button(label: &'static str) -> Stateful<gpui::Div> {
    div()
        .id("player-transport")
        .cursor_pointer()
        .w(px(44.))
        .h(px(44.))
        .rounded_full()
        .bg(rgb(0x3478e5))
        .text_color(white())
        .flex()
        .items_center()
        .justify_center()
        .text_lg()
        .hover(|style| style.bg(rgb(0x2a63c4)))
        .child(label)
}

#[cfg(target_os = "macos")]
fn player_pill(label: &str) -> Stateful<gpui::Div> {
    div()
        .id("player-pill")
        .cursor_pointer()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(white())
        .border_1()
        .border_color(rgb(BORDER))
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(TEXT))
        .hover(|style| style.bg(rgb(0xeaf2ff)))
        .child(label.to_owned())
}

#[cfg(target_os = "macos")]
fn seconds_since_midnight(date: &str) -> f64 {
    let time = date.split(' ').nth(1).unwrap_or("");
    let mut parts = time.split(':');
    let hours: f64 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.0);
    let minutes: f64 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.0);
    hours * 3600.0 + minutes * 60.0
}

fn about_section(title: &'static str, text: &'static str) -> impl IntoElement {
    div()
        .w_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_2()
        .rounded_xl()
        .bg(white())
        .border_1()
        .border_color(rgb(BORDER))
        .shadow_sm()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(title),
        )
        .child(div().text_sm().text_color(rgb(MUTED)).child(text))
}

fn render_status(status: &Status) -> impl IntoElement {
    div()
        .w_full()
        .max_w(px(760.))
        .p_3()
        .rounded_md()
        .text_sm()
        .when(status.error, |container| {
            container.bg(rgb(0xffecec)).text_color(rgb(0xa42b35))
        })
        .when(!status.error, |container| {
            container.bg(rgb(0xe9f7ef)).text_color(rgb(0x247147))
        })
        .child(ellipsize(&status.text, 240))
}

fn key_color(value: &str) -> u32 {
    let value = value.trim().trim_start_matches('#');
    if value.len() == 6 {
        u32::from_str_radix(value, 16).unwrap_or(BLUE)
    } else {
        match value.to_lowercase().as_str() {
            "red" => 0xd94b59,
            "green" => 0x2c9b69,
            "orange" => 0xdd8535,
            "purple" => 0x7959b8,
            "gray" | "grey" => 0x738197,
            _ => BLUE,
        }
    }
}

fn format_count(count: usize, one: &str, few: &str, many: &str) -> String {
    let suffix = if (11..=14).contains(&(count % 100)) {
        many
    } else {
        match count % 10 {
            1 => one,
            2..=4 => few,
            _ => many,
        }
    };
    format!("{count} {suffix}")
}

fn ellipsize(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

const ARCHIVE_INPUT_FORMAT: &[time::format_description::FormatItem<'static>] =
    time::macros::format_description!("[day].[month].[year] [hour]:[minute]");
const ARCHIVE_API_FORMAT: &[time::format_description::FormatItem<'static>] =
    time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

fn archive_now() -> PrimitiveDateTime {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    PrimitiveDateTime::new(now.date(), now.time())
}

fn archive_now_string() -> String {
    archive_now()
        .format(ARCHIVE_INPUT_FORMAT)
        .unwrap_or_default()
}

fn archive_api_date(input: &str, days: u32) -> Result<String, String> {
    let selected = PrimitiveDateTime::parse(input.trim(), ARCHIVE_INPUT_FORMAT)
        .map_err(|_| "Введите дату и время в формате ДД.ММ.ГГГГ ЧЧ:ММ".to_owned())?;
    let now = archive_now();
    if selected > now {
        return Err("Дата архива не может быть в будущем".to_owned());
    }
    if selected < now - time::Duration::days(i64::from(days)) {
        return Err(format!("Архив доступен только за последние {days} дн."));
    }
    selected
        .format(ARCHIVE_API_FORMAT)
        .map_err(|_| "Не удалось подготовить дату архива".to_owned())
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys(input::bindings());
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let bounds = Bounds::centered(None, size(px(1180.), px(800.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("D-KEYS Desktop".into()),
                        ..Default::default()
                    }),
                    app_id: Some("ru.d-keys.desktop".to_owned()),
                    ..Default::default()
                },
                |_, cx| cx.new(DKeysApp::new),
            )
            .expect("D-KEYS window");
        window
            .update(cx, |app, window, cx| {
                let weak = cx.weak_entity();
                let phone_weak = weak.clone();
                app.phone.update(cx, |input, _| {
                    input.set_on_submit(move |_, cx| {
                        let weak = phone_weak.clone();
                        cx.defer(move |cx| {
                            weak.update(cx, |app, cx| app.request_pin(cx)).ok();
                        });
                    });
                });
                let pin_weak = weak.clone();
                app.pin.update(cx, |input, _| {
                    input.set_on_submit(move |_, cx| {
                        let weak = pin_weak.clone();
                        cx.defer(move |cx| {
                            weak.update(cx, |app, cx| app.verify_pin(cx)).ok();
                        });
                    });
                });
                match &app.screen {
                    Screen::Phone => window.focus(&app.phone.focus_handle(cx)),
                    Screen::Pin => window.focus(&app.pin.focus_handle(cx)),
                    Screen::Keys => {
                        window.focus(&app.search.focus_handle(cx));
                        app.refresh_keys(cx);
                    }
                    Screen::Cameras
                    | Screen::CameraView(_)
                    | Screen::ArchivePicker(_)
                    | Screen::ArchiveView { .. } => app.refresh_keys(cx),
                    Screen::About => {}
                }
            })
            .expect("window initialized");
        cx.activate(true);
    });
}
