use std::{collections::BTreeMap, time::Duration};

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;

use crate::model::{
    ApiEnvelope, ArchiveLink, AuthData, Broadcast, Key, MqttData, OpenResult, OptionData,
    ServerData, Session, VideoLink,
};

const DEFAULT_MDE_URL: &str = "https://mde.s-m-d.ru:38257/DKeys_app_3/MAIN.php/";
const DEFAULT_CMDC_URL: &str = "https://cmdc.s-m-d.ru:8626";
const PROTOCOL_VERSION: &str = "5.3.14";

#[derive(Clone)]
pub struct DkeysApi {
    client: Client,
    mde_url: String,
}

impl DkeysApi {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .use_native_tls()
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("DKeys-Desktop/0.1.0")
            .build()
            .context("Не удалось создать HTTP-клиент")?;
        Ok(Self {
            client,
            mde_url: std::env::var("DKEYS_MDE_URL").unwrap_or_else(|_| DEFAULT_MDE_URL.to_owned()),
        })
    }

    pub fn request_pin(&self, phone: &str) -> Result<()> {
        let params = self.params("regPhone", phone, "");
        let response = self.post_form(&self.mde_url, &params)?;
        let envelope: ApiEnvelope<serde_json::Value> = decode(response)?;
        ensure_success(&envelope)?;
        Ok(())
    }

    pub fn verify_pin(&self, phone: &str, pin: &str) -> Result<Session> {
        let mut params = self.params("sendPin", phone, "");
        params.insert("otp".to_owned(), pin.to_owned());
        let response = self.post_form(&self.mde_url, &params)?;
        let envelope: ApiEnvelope<AuthData> = decode(response)?;
        ensure_success(&envelope)?;
        let data = envelope
            .data
            .ok_or_else(|| anyhow!("Сервер не вернул токен авторизации"))?;
        Ok(Session {
            phone: phone.to_owned(),
            token: data.token,
            cmdc_url: DEFAULT_CMDC_URL.to_owned(),
            l3_url: "https://vrs.s-m-d.ru:8631".to_owned(),
            l6_url: "https://vcmv.s-m-d.ru:8676".to_owned(),
        })
    }

    pub fn refresh_servers(&self, session: &mut Session) -> Result<()> {
        let params = self.params("getServers", &session.phone, &session.token);
        let response = self.post_form(&self.mde_url, &params)?;
        let envelope: ApiEnvelope<Vec<ServerData>> = decode(response)?;
        ensure_success(&envelope)?;
        if let Some(servers) = envelope.data {
            for server in servers {
                let domain = server.domain.trim_end_matches('/').to_owned();
                match server.server.as_str() {
                    "L2" => session.cmdc_url = domain,
                    "l3" => session.l3_url = domain,
                    "L6" => session.l6_url = domain,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub fn load_keys(&self, session: &Session) -> Result<Vec<Key>> {
        let params = self.params("getKey", &session.phone, &session.token);
        let response = self.post_form(&self.mde_url, &params)?;
        let envelope: ApiEnvelope<Vec<Key>> = decode(response)?;
        ensure_success(&envelope)?;
        let mut keys = envelope.data.unwrap_or_default();
        keys.sort_by(|left, right| {
            let left_index = left.index_number.parse::<u64>().unwrap_or(u64::MAX);
            let right_index = right.index_number.parse::<u64>().unwrap_or(u64::MAX);
            left_index
                .cmp(&right_index)
                .then_with(|| left.title.cmp(&right.title))
        });
        Ok(keys)
    }

    pub fn load_broadcasts(&self, session: &Session) -> Result<Vec<Broadcast>> {
        let params = self.params("getVideo", &session.phone, &session.token);
        let response = self.post_form(&self.mde_url, &params)?;
        let envelope: ApiEnvelope<Vec<Broadcast>> = decode(response)?;
        ensure_success(&envelope)?;
        let mut broadcasts = envelope.data.unwrap_or_default();
        broadcasts.sort_by(|left, right| {
            let left_index = left.index_number.parse::<u64>().unwrap_or(u64::MAX);
            let right_index = right.index_number.parse::<u64>().unwrap_or(u64::MAX);
            left_index
                .cmp(&right_index)
                .then_with(|| left.nickname.cmp(&right.nickname))
        });
        Ok(broadcasts)
    }

    pub fn video_link(&self, session: &Session, broadcast: &Broadcast) -> Result<VideoLink> {
        let protocol = if broadcast.protocol.contains("WebRTC") {
            "WebRTC"
        } else {
            "RTSP"
        };
        match self.video_link_with_protocol(session, broadcast, protocol) {
            Ok(link) => Ok(link),
            Err(web_error) if protocol == "WebRTC" => self
                .video_link_with_protocol(session, broadcast, "RTSP")
                .map_err(|rtsp_error| anyhow!("WebRTC: {web_error}; RTSP: {rtsp_error}")),
            Err(error) => Err(error),
        }
    }

    fn video_link_with_protocol(
        &self,
        session: &Session,
        broadcast: &Broadcast,
        protocol: &str,
    ) -> Result<VideoLink> {
        let server_url = if matches!(broadcast.server.as_str(), "L3" | "l3") {
            &session.l3_url
        } else {
            &session.l6_url
        };
        let mut params = self.params("getVideoBroadcast", &session.phone, &session.token);
        params.insert("codeMP".to_owned(), broadcast.code_mp.clone());
        params.insert("protocol".to_owned(), protocol.to_owned());
        let response = self.post_json(
            &format!(
                "{}/app/getVideoBroadcastByProtocol",
                server_url.trim_end_matches('/')
            ),
            &params,
        )?;
        let envelope: ApiEnvelope<VideoLink> = decode(response)?;
        ensure_success(&envelope)?;
        if let Some(warning) = envelope
            .warning
            .as_ref()
            .filter(|text| !text.trim().is_empty())
        {
            return Err(anyhow!(warning.clone()));
        }
        envelope
            .data
            .filter(|link| !link.url.is_empty())
            .ok_or_else(|| anyhow!("Сервер не вернул ссылку на видеопоток"))
    }

    pub fn online_archive_link(
        &self,
        session: &Session,
        broadcast: &Broadcast,
        date_view: &str,
    ) -> Result<ArchiveLink> {
        let server_url = if matches!(broadcast.server.as_str(), "L3" | "l3") {
            &session.l3_url
        } else {
            &session.l6_url
        };
        let mut params = self.params("getLinkVideoArchiveOnline", &session.phone, &session.token);
        params.insert("codeMP".to_owned(), broadcast.code_mp.clone());
        params.insert("dateView".to_owned(), date_view.to_owned());
        let response = self.post_json(
            &format!(
                "{}/app/getLinkVideoArchiveOnline",
                server_url.trim_end_matches('/')
            ),
            &params,
        )?;
        let envelope: ApiEnvelope<ArchiveLink> = decode(response)?;
        ensure_success(&envelope)?;
        if let Some(warning) = envelope
            .warning
            .as_ref()
            .filter(|text| !text.trim().is_empty())
        {
            return Err(anyhow!(warning.clone()));
        }
        envelope
            .data
            .filter(|link| !link.link.is_empty() && !link.task.is_empty())
            .ok_or_else(|| anyhow!("Сервер не вернул ссылку на онлайн-архив"))
    }

    pub fn close_archive_session(&self, task: &str) -> Result<()> {
        let response = self
            .client
            .delete(format!(
                "http://01playbackarh.d-keys.info:4459/api/v1/session/end/{}",
                urlencoding::encode(task)
            ))
            .send()
            .context("Не удалось закрыть сессию онлайн-архива")?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(anyhow!(
                "Сервер не подтвердил закрытие сессии архива: HTTP {}",
                response.status()
            ))
        }
    }

    pub fn open_door(&self, session: &Session, key: &Key) -> Result<OpenResult> {
        let (topic, command) = self.door_parameters(session, key)?;
        if topic.is_empty() || command.is_empty() {
            return Err(anyhow!(
                "Для этого ключа сервер не вернул MQTT topic и команду открытия"
            ));
        }
        let url = format!(
            "{}/openDoor",
            std::env::var("DKEYS_CMDC_URL")
                .unwrap_or_else(|_| session.cmdc_url.clone())
                .trim_end_matches('/')
        );
        let mut params = self.params("openDoor", &session.phone, &session.token);
        params.insert("topic".to_owned(), topic.clone());
        params.insert("command".to_owned(), command);
        params.insert(
            "uid".to_owned(),
            topic
                .split_once('/')
                .map(|(_, uid)| uid)
                .unwrap_or("")
                .to_owned(),
        );
        let response = self.post_json(&url, &params)?;
        let envelope: ApiEnvelope<serde_json::Value> = decode(response)?;
        ensure_success(&envelope)?;
        Ok(OpenResult {
            message: envelope
                .message
                .clone()
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| "Команда открытия отправлена".to_owned()),
            warning: envelope.warning.clone(),
        })
    }

    pub fn resolve_mqtt(&self, session: &Session, key: &Key) -> Result<Option<MqttData>> {
        if key.mqtt_option.is_empty() {
            return Ok(None);
        }
        let mut params = self.params("getDataOption", &session.phone, &session.token);
        params.insert("option".to_owned(), "mqtt".to_owned());
        params.insert("code".to_owned(), key.mqtt_option.clone());
        let response = self.post_form(&self.mde_url, &params)?;
        let envelope: ApiEnvelope<OptionData> = decode(response)?;
        ensure_success(&envelope)?;
        Ok(envelope.data.and_then(|data| data.mqtt))
    }

    pub fn logout(&self, session: &Session) -> Result<()> {
        let params = self.params("logOut", &session.phone, &session.token);
        let response = self.post_form(&self.mde_url, &params)?;
        let envelope: ApiEnvelope<serde_json::Value> = decode(response)?;
        ensure_success(&envelope)?;
        Ok(())
    }

    fn door_parameters(&self, session: &Session, key: &Key) -> Result<(String, String)> {
        if !key.topic.is_empty() && !key.command.is_empty() {
            return Ok((key.topic.clone(), key.command.clone()));
        }
        if let Some(mqtt) = self.resolve_mqtt(session, key)? {
            return Ok((mqtt.topic, mqtt.command));
        }
        Ok((key.topic.clone(), key.command.clone()))
    }

    fn params(&self, action: &str, phone: &str, token: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("action".to_owned(), action.to_owned()),
            ("os".to_owned(), "Android".to_owned()),
            ("lang".to_owned(), "ru".to_owned()),
            ("version".to_owned(), PROTOCOL_VERSION.to_owned()),
            ("token".to_owned(), token.to_owned()),
            ("mobile".to_owned(), phone.to_owned()),
            ("versionOS".to_owned(), "36".to_owned()),
        ])
    }

    fn post_form(&self, url: &str, params: &BTreeMap<String, String>) -> Result<Response> {
        self.client
            .post(url)
            .form(params)
            .send()
            .with_context(|| format!("Не удалось подключиться к {url}"))
    }

    fn post_json(&self, url: &str, params: &BTreeMap<String, String>) -> Result<Response> {
        self.client
            .post(url)
            .json(params)
            .send()
            .with_context(|| format!("Не удалось подключиться к {url}"))
    }
}

fn decode<T: DeserializeOwned>(response: Response) -> Result<ApiEnvelope<T>> {
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("Сервер вернул HTTP {status}"));
    }
    response
        .json::<ApiEnvelope<T>>()
        .context("Сервер вернул неизвестный формат ответа")
}

fn ensure_success<T>(envelope: &ApiEnvelope<T>) -> Result<()> {
    if envelope.error {
        return Err(anyhow!(
            envelope
                .error_message()
                .unwrap_or_else(|| "Сервер отклонил запрос".to_owned())
        ));
    }
    Ok(())
}

pub fn normalize_phone(input: &str) -> String {
    let digits: String = input.chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 10 {
        format!("7{digits}")
    } else if digits.len() == 11 && digits.starts_with('8') {
        format!("7{}", &digits[1..])
    } else {
        digits
    }
}
