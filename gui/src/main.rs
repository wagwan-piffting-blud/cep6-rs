//! Ceps: text box, voice controls and a markup menu over the `ceps` library.
//! Synthesis runs in-process on a blocking thread; the webview only edits
//! text and plays the WAV bytes it gets back.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ceps::render::{Options, Report};
use ceps::{CepLex, PosTag, Voice};
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{Manager, State};

const CEPLEX: &[u8] = include_bytes!("../../packs/ceplex.bin");
const POS: &[u8] = include_bytes!("../../packs/pos.bin");

struct Engine {
    cep: CepLex,
    pos: Option<PosTag>,
    /// Where the search may look besides a folder the user picked.
    roots: Vec<PathBuf>,
    voices: Mutex<Vec<PathBuf>>,
    /// Only the voice in use stays loaded: together they are ~440 MB.
    loaded: Mutex<Option<(PathBuf, Arc<Voice>)>>,
    last: Mutex<Notes>,
}

#[derive(Deserialize)]
struct Request {
    text: String,
    voice: String,
    markup: bool,
    rate: i32,
    pitch: u32,
    volume: u32,
}

#[derive(Serialize, Clone, Default)]
struct Notes {
    seconds: f32,
    ignored: Vec<String>,
    missing: Vec<String>,
}

#[derive(Serialize)]
struct Check {
    error: Option<String>,
    ignored: Vec<String>,
}

#[derive(Serialize)]
struct VoiceList {
    names: Vec<String>,
    /// The folder they were found in.
    root: Option<String>,
}

/// Where voices are looked for when the user has not picked a folder:
/// $CEPS_VOICE_ROOT, the bundle's `voices` (absent from a voiceless build),
/// a Cepstral Swift install, then (in a debug build) the repo's own copies.
fn default_roots(app: &tauri::App) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(r) = std::env::var("CEPS_VOICE_ROOT") {
        roots.push(r.into());
    }
    if let Ok(r) = app.path().resource_dir() {
        roots.push(r.join("voices"));
    }
    roots.extend(ceps::install::voice_roots());
    if cfg!(debug_assertions) {
        roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    }
    roots
}

/// The voices in `root`: `root` itself if it is a voice folder, else each
/// folder in it that holds a voice.idx.
fn scan(root: &Path) -> Vec<PathBuf> {
    if root.join("voice.idx").is_file() {
        return vec![root.to_path_buf()];
    }
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("voice.idx").is_file())
        .collect();
    found.sort();
    found
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Engine {
    fn voice(&self, name: &str) -> Result<Arc<Voice>, String> {
        let dir = self.voices.lock().unwrap().iter().find(|p| name_of(p) == name).cloned()
            .ok_or_else(|| format!("no voice named {name}"))?;
        let dir = &dir;
        let mut loaded = self.loaded.lock().unwrap();
        if let Some((d, v)) = loaded.as_ref() {
            if d == dir {
                return Ok(v.clone());
            }
        }
        // drop the old one before reading the new
        *loaded = None;
        let v = Arc::new(Voice::open(dir).map_err(|e| format!("cannot open {}: {e}", dir.display()))?);
        *loaded = Some((dir.clone(), v.clone()));
        Ok(v)
    }

    fn render_each<F: FnMut(&[i16])>(&self, r: &Request, each: F) -> Result<(u32, Report), String> {
        let v = self.voice(&r.voice)?;
        let o = Options {
            markup: r.markup,
            rate: r.rate.clamp(-10, 10),
            pitch_shift: r.pitch.clamp(50, 200) as f32 / 100.0,
            volume: r.volume.min(100),
            ..Options::default()
        };
        let report = ceps::render::render_each(&v, &self.cep, self.pos.as_ref(), &r.text, &o, each)
            .map_err(|e| format!("markup: {e}"))?;
        Ok((v.sps, report))
    }

    fn remember(&self, sps: u32, r: Report) -> Notes {
        let n = Notes { seconds: r.samples as f32 / sps as f32, ignored: r.ignored, missing: r.missing };
        *self.last.lock().unwrap() = n.clone();
        n
    }
}

fn wav_bytes(pcm: &[i16], sps: u32) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&sps.to_le_bytes());
    b.extend_from_slice(&(sps * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// Find the voices: in `root` if the user picked one (and it holds any),
/// else in the first default place that does.
#[tauri::command]
fn voices(engine: State<'_, Arc<Engine>>, root: Option<String>) -> VoiceList {
    let picked = root.map(PathBuf::from);
    let (found, from) = picked.iter().chain(engine.roots.iter())
        .map(|r| (scan(r), r))
        .find(|(v, _)| !v.is_empty())
        .map_or((Vec::new(), None), |(v, r)| (v, Some(r.display().to_string())));
    let names = found.iter().map(|p| name_of(p)).collect();
    *engine.voices.lock().unwrap() = found;
    VoiceList { names, root: from }
}

/// The WAV for `req`, as raw bytes. `notes` has what it left out.
#[tauri::command]
async fn speak(engine: State<'_, Arc<Engine>>, req: Request) -> Result<Response, String> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut pcm = Vec::new();
        let (sps, report) = engine.render_each(&req, |p| pcm.extend_from_slice(p))?;
        engine.remember(sps, report);
        Ok(Response::new(wav_bytes(&pcm, sps)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Render straight to a file an utterance at a time, so length is no limit.
#[tauri::command]
async fn save(engine: State<'_, Arc<Engine>>, req: Request, path: String) -> Result<Notes, String> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let sps = engine.voice(&req.voice)?.sps;
        let mut w = ceps::wav::Writer::create(&path, sps).map_err(|e| format!("cannot write {path}: {e}"))?;
        let mut failed = None;
        let (sps, report) = engine.render_each(&req, |p| {
            if failed.is_none() {
                failed = w.push(p).err();
            }
        })?;
        if let Some(e) = failed {
            return Err(format!("writing {path} failed: {e}"));
        }
        w.finish().map_err(|e| format!("cannot finish {path}: {e}"))?;
        Ok(engine.remember(sps, report))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn notes(engine: State<'_, Arc<Engine>>) -> Notes {
    engine.last.lock().unwrap().clone()
}

/// Parse errors and ignored tags, before anything is rendered.
#[tauri::command]
fn check(text: String) -> Check {
    match ceps::markup::parse(&text) {
        Ok(m) => Check { error: None, ignored: m.ignored },
        Err(e) => Check { error: Some(e.to_string()), ignored: Vec::new() },
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cep = CepLex::from_bytes(CEPLEX.to_vec()).expect("the built-in lexicon is damaged");
            let engine = Engine {
                cep,
                pos: PosTag::from_bytes(POS.to_vec()),
                roots: default_roots(app),
                voices: Mutex::new(Vec::new()),
                loaded: Mutex::new(None),
                last: Mutex::new(Notes::default()),
            };
            app.manage(Arc::new(engine));

            #[allow(unused_mut)]
            let mut window = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Ceps")
                .inner_size(760.0, 620.0)
                .min_inner_size(560.0, 460.0);
            // Tauri's own WebView2 arguments replace
            // WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, so a DevTools port for
            // tools/cdp.py has to come through here. The first three are
            // Tauri's defaults.
            #[cfg(windows)]
            if let Ok(port) = std::env::var("CEPS_DEVTOOLS_PORT") {
                window = window.additional_browser_args(&format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                     --remote-debugging-port={port}"));
            }
            window.build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![voices, speak, save, notes, check])
        .run(tauri::generate_context!())
        .expect("error while running Ceps");
}
