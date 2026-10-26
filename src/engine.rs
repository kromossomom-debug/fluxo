//! yt-dlp (extraction/downloading) and ffmpeg (merging/converting) integration.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::i18n::{fill, tr};

#[cfg(windows)]
const YTDLP_NAME: &str = "yt-dlp.exe";
#[cfg(not(windows))]
const YTDLP_NAME: &str = "yt-dlp";

#[cfg(windows)]
const FFMPEG_NAME: &str = "ffmpeg.exe";
#[cfg(not(windows))]
const FFMPEG_NAME: &str = "ffmpeg";

#[cfg(windows)]
const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
#[cfg(not(windows))]
const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux";

#[cfg(windows)]
const FFMPEG_URL: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip";
#[cfg(not(windows))]
const FFMPEG_URL: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-linux64-gpl.tar.xz";

/// Folder where Fluxo keeps the tools it downloads itself.
pub fn tools_dir() -> PathBuf {
    crate::store::data_dir().join("bin")
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let mut candidates = vec![tools_dir().join(name)];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).map(|p| p.join(name)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn ytdlp_path() -> Option<PathBuf> {
    find_tool(YTDLP_NAME)
}

pub fn ffmpeg_path() -> Option<PathBuf> {
    find_tool(FFMPEG_NAME)
}

fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn ytdlp_command() -> Result<Command, String> {
    let path = ytdlp_path().ok_or(tr().ytdlp_missing)?;
    let mut cmd = command(&path);
    cmd.args(["--encoding", "utf-8", "--no-warnings", "--no-config"]);
    if let Some(ff) = ffmpeg_path() {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
    Ok(cmd)
}

pub fn ytdlp_version() -> Option<String> {
    let out = command(&ytdlp_path()?).arg("--version").output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run_json(mut cmd: Command) -> Result<serde_json::Value, String> {
    let out = cmd.output().map_err(|e| fill(tr().run_failed, e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(tr().unknown_error);
        return Err(line.trim_start_matches("ERROR: ").to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| fill(tr().invalid_response, e))
}

// ---------------------------------------------------------------------------
// Metadata

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub id: String,
    pub title: String,
    pub url: String,
    pub uploader: String,
    pub thumbnail: String,
    pub duration: Option<f64>,
    pub views: Option<u64>,
    pub site: String,
    /// Available video heights (e.g. 2160, 1080, 720), largest first.
    pub heights: Vec<u32>,
    /// Approximate size in bytes per height, when the site reports it.
    pub sizes: Vec<(u32, u64)>,
    pub audio_size: Option<u64>,
    /// true when it came from a search (formats not loaded yet).
    pub partial: bool,
}

#[derive(Deserialize, Default)]
struct RawFormat {
    height: Option<u32>,
    vcodec: Option<String>,
    acodec: Option<String>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
}

#[derive(Deserialize, Default)]
struct RawThumb {
    url: Option<String>,
    width: Option<u32>,
}

#[derive(Deserialize, Default)]
struct RawInfo {
    id: Option<String>,
    title: Option<String>,
    webpage_url: Option<String>,
    url: Option<String>,
    original_url: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    thumbnail: Option<String>,
    #[serde(default)]
    thumbnails: Vec<RawThumb>,
    duration: Option<f64>,
    view_count: Option<u64>,
    extractor_key: Option<String>,
    ie_key: Option<String>,
    #[serde(default)]
    formats: Vec<RawFormat>,
}

impl RawInfo {
    fn into_media(self, partial: bool) -> MediaInfo {
        let id = self.id.unwrap_or_default();
        let site = self.extractor_key.or(self.ie_key).unwrap_or_default();
        let url = self
            .webpage_url
            .or(self.original_url)
            .or(self.url)
            .unwrap_or_default();
        let thumbnail = if site.starts_with("Youtube") && !id.is_empty() {
            format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
        } else {
            self.thumbnail
                .or_else(|| {
                    self.thumbnails
                        .iter()
                        .filter(|t| t.url.is_some())
                        .max_by_key(|t| t.width.unwrap_or(0).min(640))
                        .and_then(|t| t.url.clone())
                })
                .unwrap_or_default()
        };

        let mut heights: Vec<u32> = Vec::new();
        let mut sizes: Vec<(u32, u64)> = Vec::new();
        let mut audio_size = None;
        for f in &self.formats {
            let size = f.filesize.or(f.filesize_approx);
            let has_video = f.vcodec.as_deref().is_some_and(|v| v != "none");
            let has_audio = f.acodec.as_deref().is_some_and(|a| a != "none");
            match f.height {
                Some(h) if has_video && h >= 144 => {
                    if !heights.contains(&h) {
                        heights.push(h);
                    }
                    if let Some(s) = size {
                        match sizes.iter_mut().find(|(sh, _)| *sh == h) {
                            Some(entry) => entry.1 = entry.1.max(s),
                            None => sizes.push((h, s)),
                        }
                    }
                }
                _ if has_audio && !has_video => {
                    if let Some(s) = size {
                        audio_size = Some(audio_size.map_or(s, |a: u64| a.max(s)));
                    }
                }
                _ => {}
            }
        }
        heights.sort_unstable_by(|a, b| b.cmp(a));

        MediaInfo {
            id,
            title: self.title.unwrap_or_else(|| tr().untitled.into()),
            url,
            uploader: self.uploader.or(self.channel).unwrap_or_default(),
            thumbnail,
            duration: self.duration,
            views: self.view_count,
            site,
            heights,
            sizes,
            audio_size,
            partial,
        }
    }
}

/// Fetches full metadata (including formats) for a link.
pub fn fetch_info(url: &str) -> Result<MediaInfo, String> {
    let mut cmd = ytdlp_command()?;
    cmd.args(["-J", "--no-playlist", "--", url]);
    let value = run_json(cmd)?;
    let raw: RawInfo = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok(raw.into_media(false))
}

/// Searches YouTube.
pub fn search(query: &str, count: u32) -> Result<Vec<MediaInfo>, String> {
    #[derive(Deserialize)]
    struct Playlist {
        #[serde(default)]
        entries: Vec<RawInfo>,
    }
    let mut cmd = ytdlp_command()?;
    cmd.args(["-J", "--flat-playlist"])
        .arg(format!("ytsearch{count}:{query}"));
    let value = run_json(cmd)?;
    let list: Playlist = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok(list
        .entries
        .into_iter()
        .map(|mut e| {
            if e.extractor_key.is_none() && e.ie_key.is_none() {
                e.ie_key = Some("Youtube".into());
            }
            e.into_media(true)
        })
        .filter(|m| !m.id.is_empty())
        .collect())
}

// ---------------------------------------------------------------------------
// Download

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Quality {
    /// Best video up to the given height.
    Video(u32),
    Mp3,
    M4a,
}

impl Quality {
    pub fn label(self) -> String {
        match self {
            Quality::Video(h) => format!("{} · MP4", height_label(h)),
            Quality::Mp3 => format!("{} · MP3", tr().audio),
            Quality::M4a => format!("{} · M4A", tr().audio),
        }
    }
    pub fn is_audio(self) -> bool {
        !matches!(self, Quality::Video(_))
    }
}

pub fn height_label(h: u32) -> String {
    match h {
        4320.. => "8K".into(),
        2160.. => "4K".into(),
        1440.. => "2K".into(),
        _ => format!("{h}p"),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub fraction: f32,
    pub speed: String,
    pub eta: String,
    pub total: String,
    pub stage: String,
}

pub enum Event {
    Progress(Progress),
    Done(PathBuf),
    Failed(String),
}

pub type ChildSlot = Arc<Mutex<Option<Child>>>;

fn clean(s: &str) -> String {
    let s = s.trim();
    if s == "NA" || s == "Unknown" || s.starts_with("Unknown") {
        String::new()
    } else {
        s.to_string()
    }
}

/// Runs the download and calls `on_event` (on its own thread) for every update.
pub fn download(
    url: &str,
    quality: Quality,
    out_dir: &Path,
    child_slot: ChildSlot,
    mut on_event: impl FnMut(Event) + Send + 'static,
) {
    let mut cmd = match ytdlp_command() {
        Ok(c) => c,
        Err(e) => return on_event(Event::Failed(e)),
    };
    let has_ffmpeg = ffmpeg_path().is_some();
    cmd.args(["--no-playlist", "--newline", "--progress", "--no-simulate", "--no-mtime"])
        .args([
            "--progress-template",
            "download:FLX|%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(progress._total_bytes_str)s|%(progress._total_bytes_estimate_str)s",
        ])
        .args(["--print", "after_move:FLXDONE|%(filepath)s"])
        .arg("-P")
        .arg(out_dir)
        .args(["-o", "%(title).150B [%(id)s].%(ext)s"]);

    match quality {
        Quality::Video(h) if has_ffmpeg => {
            cmd.arg("-f")
                .arg(format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b"))
                .args(["--merge-output-format", "mp4"]);
        }
        Quality::Video(h) => {
            cmd.arg("-f").arg(format!(
                "b[height<={h}][vcodec!=none][acodec!=none]/b[vcodec!=none][acodec!=none]/b"
            ));
        }
        Quality::Mp3 if has_ffmpeg => {
            cmd.args(["-f", "ba/b", "-x", "--audio-format", "mp3", "--audio-quality", "0"]);
        }
        Quality::Mp3 | Quality::M4a => {
            cmd.args(["-f", "ba[ext=m4a]/ba/b"]);
        }
    }
    cmd.arg("--").arg(url);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return on_event(Event::Failed(fill(tr().start_failed, e))),
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    *child_slot.lock().unwrap() = Some(child);

    // stderr is read separately so the process never blocks; keeps the last error line.
    let last_error = Arc::new(Mutex::new(String::new()));
    let err_thread = stderr.map(|stderr| {
        let last_error = last_error.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.contains("ERROR") || !line.trim().is_empty() {
                    *last_error.lock().unwrap() = line.trim_start_matches("ERROR: ").to_string();
                }
            }
        })
    });

    let mut final_path: Option<PathBuf> = None;
    let mut part = 0u32;
    let mut last_fraction = 0.0f32;
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("FLX|") {
                let fields: Vec<&str> = rest.split('|').collect();
                let pct = fields
                    .first()
                    .and_then(|p| p.trim().trim_end_matches('%').parse::<f32>().ok())
                    .unwrap_or(0.0)
                    / 100.0;
                // A new file started (e.g. audio after video).
                if pct + 0.2 < last_fraction {
                    part += 1;
                }
                last_fraction = pct;
                let total = fields
                    .get(3)
                    .map(|s| clean(s))
                    .filter(|s| !s.is_empty())
                    .or_else(|| fields.get(4).map(|s| clean(s)))
                    .unwrap_or_default();
                on_event(Event::Progress(Progress {
                    fraction: pct.clamp(0.0, 1.0),
                    speed: fields.get(1).map(|s| clean(s)).unwrap_or_default(),
                    eta: fields.get(2).map(|s| clean(s)).unwrap_or_default(),
                    total,
                    stage: if part == 0 { tr().downloading.into() } else { fill(tr().downloading_part, part + 1) },
                }));
            } else if let Some(path) = line.strip_prefix("FLXDONE|") {
                final_path = Some(PathBuf::from(path.trim()));
            } else if line.starts_with("[Merger]") || line.starts_with("[ExtractAudio]") || line.starts_with("[VideoConvertor]") {
                on_event(Event::Progress(Progress {
                    fraction: 1.0,
                    stage: tr().processing.into(),
                    ..Default::default()
                }));
            }
        }
    }
    if let Some(t) = err_thread {
        let _ = t.join();
    }

    let status = child_slot.lock().unwrap().as_mut().map(|c| c.wait());
    *child_slot.lock().unwrap() = None;
    match (status, final_path) {
        (Some(Ok(s)), Some(path)) if s.success() => on_event(Event::Done(path)),
        (Some(Ok(s)), _) if s.success() => on_event(Event::Done(out_dir.to_path_buf())),
        _ => {
            let err = last_error.lock().unwrap().clone();
            on_event(Event::Failed(if err.is_empty() { tr().interrupted.into() } else { err }));
        }
    }
}

/// Kills the download process (and its children, on Windows).
pub fn kill(child_slot: &ChildSlot) {
    if let Some(child) = child_slot.lock().unwrap().as_mut() {
        #[cfg(windows)]
        {
            let _ = command(Path::new("taskkill"))
                .args(["/T", "/F", "/PID", &child.id().to_string()])
                .output();
        }
        let _ = child.kill();
    }
}

// ---------------------------------------------------------------------------
// Tool installation

fn http_get(url: &str, on_progress: &mut dyn FnMut(f32)) -> Result<Vec<u8>, String> {
    let mut resp = ureq::get(url).call().map_err(|e| fill(tr().network_error, e))?;
    let total: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let mut reader = resp.body_mut().with_config().limit(1 << 30).reader();
    let mut data = Vec::with_capacity(total.unwrap_or(1 << 20) as usize);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| fill(tr().network_error, e))?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(t) = total {
            on_progress(data.len() as f32 / t as f32);
        }
    }
    Ok(data)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
}
#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<(), String> {
    Ok(())
}

/// Downloads the official yt-dlp (GitHub) into Fluxo's tools folder.
pub fn install_ytdlp(mut on_progress: impl FnMut(f32)) -> Result<(), String> {
    let data = http_get(YTDLP_URL, &mut on_progress)?;
    let dir = tools_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(YTDLP_NAME);
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    make_executable(&path)
}

/// Updates yt-dlp with its own updater (`-U`).
pub fn update_ytdlp() -> Result<String, String> {
    let path = ytdlp_path().ok_or(tr().ytdlp_missing)?;
    let out = command(&path).arg("-U").output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().last().unwrap_or("").to_string())
}

/// Downloads a static ffmpeg build (yt-dlp/FFmpeg-Builds) and extracts ffmpeg/ffprobe.
pub fn install_ffmpeg(mut on_progress: impl FnMut(f32)) -> Result<(), String> {
    let data = http_get(FFMPEG_URL, &mut on_progress)?;
    let dir = tools_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wanted = |name: &str| -> Option<&'static str> {
        let file = name.rsplit('/').next()?;
        ["ffmpeg", "ffprobe", "ffmpeg.exe", "ffprobe.exe"]
            .into_iter()
            .find(|w| *w == file && name.contains("/bin/"))
    };
    let mut found = 0;

    if FFMPEG_URL.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let entry_name = file.name().map(|n| n.to_string()).unwrap_or_default();
            if let Some(name) = wanted(&entry_name) {
                let mut out = std::fs::File::create(dir.join(name)).map_err(|e| e.to_string())?;
                std::io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
                found += 1;
            }
        }
    } else {
        let mut tar_bytes = Vec::new();
        lzma_rs::xz_decompress(&mut std::io::Cursor::new(data), &mut tar_bytes)
            .map_err(|e| fill(tr().unpack_failed, e))?;
        let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
            if let Some(name) = wanted(&path) {
                let target = dir.join(name);
                let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                make_executable(&target)?;
                found += 1;
            }
        }
    }
    if found == 0 {
        return Err(tr().ffmpeg_not_in_package.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Network tests (they run the real yt-dlp): `cargo test -- --ignored`.
    use super::*;

    const SHORT_VIDEO: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

    #[test]
    #[ignore]
    fn search_returns_results() {
        let list = search("rust programming language", 5).expect("search");
        assert!(!list.is_empty());
        for m in &list {
            assert!(!m.id.is_empty() && !m.title.is_empty() && m.thumbnail.starts_with("https://"));
            println!("{} | {} | {:?}", m.title, m.uploader, m.duration);
        }
    }

    #[test]
    #[ignore]
    fn fetch_info_lists_formats() {
        let m = fetch_info(SHORT_VIDEO).expect("info");
        println!("{} | {} | {:?} | {:?}", m.title, m.site, m.heights, m.duration);
        assert!(!m.heights.is_empty());
        assert_eq!(m.site, "Youtube");
    }

    fn run_download(quality: Quality) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fluxo-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        download(SHORT_VIDEO, quality, &dir, Arc::new(Mutex::new(None)), move |ev| {
            let _ = tx.send(ev);
        });
        let mut last = 0.0;
        let mut updates = 0;
        for ev in rx.try_iter() {
            match ev {
                Event::Progress(p) => {
                    updates += 1;
                    last = p.fraction;
                }
                Event::Done(path) => {
                    println!("{quality:?}: {updates} updates, last {last:.2}, file {}", path.display());
                    assert!(path.is_file(), "final file must exist");
                    assert!(updates > 0);
                    return path;
                }
                Event::Failed(e) => panic!("failed: {e}"),
            }
        }
        panic!("no final event");
    }

    #[test]
    #[ignore]
    fn download_video_and_mp3() {
        let video = run_download(Quality::Video(360));
        assert_eq!(video.extension().unwrap(), "mp4");
        let audio = run_download(Quality::Mp3);
        assert_eq!(audio.extension().unwrap(), if ffmpeg_path().is_some() { "mp3" } else { "m4a" });
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }
}
