//! Configurações e histórico salvos em JSON na pasta de dados do usuário.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::engine::Quality;

pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("io", "Fluxo", "Fluxo")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("fluxo"))
}

fn default_download_dir() -> PathBuf {
    let user = directories::UserDirs::new();
    user.as_ref()
        .and_then(|u| u.download_dir().or(u.video_dir()))
        .map(|d| d.join("Fluxo"))
        .unwrap_or_else(|| data_dir().join("downloads"))
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub dark: bool,
    pub max_parallel: usize,
    pub default_quality: Quality,
    pub search_results: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            dark: true,
            max_parallel: 3,
            default_quality: Quality::Video(1080),
            search_results: 16,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryItem {
    pub title: String,
    pub url: String,
    pub thumbnail: String,
    pub uploader: String,
    pub path: PathBuf,
    pub quality: Quality,
    pub size: String,
    pub finished_at: u64,
}

fn load<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    std::fs::read(data_dir().join(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(name: &str, value: &T) {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_vec_pretty(value) {
        let tmp = dir.join(format!("{name}.tmp"));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, dir.join(name));
        }
    }
}

pub fn load_settings() -> Settings {
    load("settings.json")
}

pub fn save_settings(s: &Settings) {
    save("settings.json", s);
}

pub fn load_history() -> Vec<HistoryItem> {
    load("history.json")
}

pub fn save_history(h: &[HistoryItem]) {
    save("history.json", &h);
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
