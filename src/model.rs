use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub phone: String,
    pub token: String,
    #[serde(default = "default_cmdc_url")]
    pub cmdc_url: String,
    #[serde(default = "default_l3_url")]
    pub l3_url: String,
    #[serde(default = "default_l6_url")]
    pub l6_url: String,
}

fn default_cmdc_url() -> String {
    "https://cmdc.s-m-d.ru:8626".to_owned()
}

fn default_l3_url() -> String {
    "https://vrs.s-m-d.ru:8631".to_owned()
}

fn default_l6_url() -> String {
    "https://vcmv.s-m-d.ru:8676".to_owned()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Key {
    #[serde(rename = "Nickname", default)]
    pub title: String,
    #[serde(rename = "Address", default)]
    pub address: String,
    #[serde(rename = "codeKey", default)]
    pub code: String,
    #[serde(rename = "Color", default)]
    pub color: String,
    #[serde(rename = "Icon", default)]
    pub icon: String,
    #[serde(rename = "TypeKey", default)]
    pub type_key: String,
    #[serde(rename = "expirationDate", default)]
    pub expiration_date: String,
    #[serde(rename = "indexNumber", default)]
    pub index_number: String,
    #[serde(rename = "codeMP", default)]
    pub code_mp: String,
    #[serde(rename = "mqtt", default)]
    pub mqtt_option: String,
    #[serde(rename = "options_exist", default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub uid: String,
}

impl Key {
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.title.to_lowercase().contains(&query)
            || self.address.to_lowercase().contains(&query)
            || self.code.to_lowercase().contains(&query)
    }

    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            "Ключ D-KEYS"
        } else {
            &self.title
        }
    }

    pub fn display_address(&self) -> &str {
        if self.address.trim().is_empty() {
            "Адрес не указан"
        } else {
            &self.address
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthData {
    #[serde(rename = "Token")]
    pub token: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerData {
    pub server: String,
    #[serde(rename = "domen")]
    pub domain: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct OptionData {
    #[serde(default)]
    pub mqtt: Option<MqttData>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MqttData {
    #[serde(rename = "topicCommand", default)]
    pub topic: String,
    #[serde(rename = "openCommand", default)]
    pub command: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct ApiEnvelope<T> {
    #[serde(default)]
    pub error: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub warning: Option<String>,
    #[serde(default)]
    pub critical: Option<String>,
    #[serde(rename = "DATA", default)]
    pub data: Option<T>,
}

impl<T> ApiEnvelope<T> {
    pub fn error_message(&self) -> Option<String> {
        self.message
            .as_deref()
            .filter(|message| !message.trim().is_empty())
            .or(self
                .warning
                .as_deref()
                .filter(|text| !text.trim().is_empty()))
            .or(self
                .critical
                .as_deref()
                .filter(|text| !text.trim().is_empty() && !matches!(*text, "0" | "false" | "null")))
            .map(str::to_owned)
    }
}

#[derive(Clone, Debug)]
pub struct OpenResult {
    pub message: String,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Broadcast {
    #[serde(rename = "Nickname", default)]
    pub nickname: String,
    #[serde(rename = "Address", default)]
    pub address: String,
    #[serde(rename = "codeKey", default)]
    pub code: String,
    #[serde(rename = "codeMP", default)]
    pub code_mp: String,
    #[serde(rename = "Color", default)]
    pub color: String,
    #[serde(rename = "indexNumber", default)]
    pub index_number: String,
    #[serde(rename = "protocol", default)]
    pub protocol: String,
    #[serde(rename = "srv", default)]
    pub server: String,
    #[serde(rename = "UID", default)]
    pub uid: String,
    #[serde(rename = "Online", default)]
    pub online: i32,
    #[serde(rename = "recordStoragePeriod", default)]
    pub record_storage_period: String,
}

impl Broadcast {
    pub fn display_title(&self) -> &str {
        if self.nickname.trim().is_empty() {
            "Камера D-KEYS"
        } else {
            &self.nickname
        }
    }

    pub fn display_address(&self) -> &str {
        if self.address.trim().is_empty() {
            "Адрес не указан"
        } else {
            &self.address
        }
    }

    pub fn archive_days(&self) -> Option<u32> {
        self.record_storage_period
            .parse::<u32>()
            .ok()
            .filter(|days| *days > 0)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ArchiveLink {
    #[serde(default)]
    pub link: String,
    // Логин/пароль и authenticated_url() используются только в не-macOS
    // ветке открытия архива (системный плеер); на macOS URL валидируется
    // напрямую перед созданием AVPlayer.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    #[serde(default)]
    pub login: String,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    #[serde(default)]
    pub password: String,
    #[serde(rename = "result", default)]
    pub _result: String,
    #[serde(default)]
    pub task: String,
}

impl ArchiveLink {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn authenticated_url(&self) -> String {
        let url = if self.link.contains("://") {
            self.link.clone()
        } else {
            format!("http://{}", self.link)
        };
        if self.login.is_empty() || url.contains('@') {
            return url;
        }
        if let Some((scheme, address)) = url.split_once("://") {
            format!(
                "{scheme}://{}:{}@{address}",
                urlencoding::encode(&self.login),
                urlencoding::encode(&self.password)
            )
        } else {
            url
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct VideoLink {
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub url: String,
}

impl VideoLink {
    pub fn authenticated_url(&self) -> String {
        let url = if self.url.contains("://") {
            self.url.clone()
        } else if self.protocol.contains("WebRTC") {
            format!("https://{}", self.url)
        } else {
            format!("rtsp://{}", self.url)
        };
        if self.login.is_empty() || url.contains('@') {
            return url;
        }
        if let Some((scheme, address)) = url.split_once("://") {
            format!(
                "{scheme}://{}:{}@{address}",
                urlencoding::encode(&self.login),
                urlencoding::encode(&self.password)
            )
        } else {
            url
        }
    }
}
