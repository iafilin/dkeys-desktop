use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;

use crate::model::{Key, Session};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Cache {
    session: Option<Session>,
    #[serde(default)]
    keys: Vec<Key>,
}

#[derive(Clone)]
pub struct Storage {
    path: PathBuf,
}

impl Storage {
    pub fn new() -> Result<Self> {
        let directory = if let Some(directory) = std::env::var_os("DKEYS_CONFIG_DIR") {
            PathBuf::from(directory)
        } else {
            ProjectDirs::from("ru", "d-keys", "DKeys Desktop")
                .context("Не удалось определить папку настроек")?
                .config_dir()
                .to_path_buf()
        };
        fs::create_dir_all(&directory).with_context(|| {
            format!("Не удалось создать папку настроек {}", directory.display())
        })?;
        Ok(Self {
            path: directory.join("session.json"),
        })
    }

    pub fn load(&self) -> (Option<Session>, Vec<Key>) {
        let Ok(contents) = fs::read_to_string(&self.path) else {
            return (None, Vec::new());
        };
        let Ok(cache) = serde_json::from_str::<Cache>(&contents) else {
            return (None, Vec::new());
        };
        (cache.session, cache.keys)
    }

    pub fn save(&self, session: Option<&Session>, keys: &[Key]) -> Result<()> {
        let cache = Cache {
            session: session.cloned(),
            keys: keys.to_vec(),
        };
        let contents = serde_json::to_string_pretty(&cache)?;
        let temporary = self.path.with_extension("tmp");
        fs::write(&temporary, contents).context("Не удалось сохранить сессию")?;
        fs::rename(&temporary, &self.path).context("Не удалось применить сессию")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .context("Не удалось защитить файл сессии")?;
        }
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("Не удалось удалить сессию"),
        }
    }
}
