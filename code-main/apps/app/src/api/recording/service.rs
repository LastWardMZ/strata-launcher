//! Servicio de grabación (Windows): estado compartido, vigilante del juego, tecla global y comandos de Tauri.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use super::capture::Active;
use super::ffmpeg::{self, EncoderStatus};
use super::hotkey::Hotkeys;
use super::library::{self, MetaCache, Source, VideoItem};
use super::overlay::Overlay;
use super::settings::{self, Hotkey, RecordingSettings};
use super::trim;

/// Minecraft lanzado desde Strata.
#[derive(Clone, Debug)]
pub struct Game {
    pub pid: u32,
    pub instance_id: String,
    pub instance_name: String,
}

#[derive(Clone, Serialize, Debug, Default)]
pub struct HotkeyStatus {
    pub label: String,
    pub registered: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct FfmpegStatus {
    pub installed: bool,
    pub version: &'static str,
    pub installing: bool,
    pub download_mb: u64,
}

#[derive(Serialize)]
pub struct RecordingInfo {
    pub instance_name: String,
    pub elapsed_seconds: u64,
    pub encoder: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct Status {
    pub supported: bool,
    pub settings: RecordingSettings,
    pub ffmpeg: FfmpegStatus,
    pub encoders: Option<Vec<EncoderStatus>>,
    pub encoder_in_use: Option<&'static str>,
    pub probing_encoders: bool,
    pub recording: Option<RecordingInfo>,
    pub hotkey: HotkeyStatus,
    pub game_running: bool,
}

pub struct Service {
    pub(super) app: tauri::AppHandle,
    pub(super) dir: PathBuf,
    pub(super) settings: Mutex<RecordingSettings>,
    pub(super) encoders: tokio::sync::Mutex<Option<Vec<EncoderStatus>>>,
    pub(super) probing: AtomicBool,
    pub(super) installing: AtomicBool,
    /// `true` mientras hay una grabación en curso (lo lee la tecla, que es síncrona).
    pub(super) recording: AtomicBool,
    pub(super) active: tokio::sync::Mutex<Option<Active>>,
    pub(super) games: Mutex<Vec<Game>>,
    pub(super) hotkey_status: Mutex<HotkeyStatus>,
    registered_for: Mutex<Option<Hotkey>>,
    pub(super) hotkeys: Hotkeys,
    pub(super) overlay: Overlay,
    pub(super) trims: Mutex<HashMap<String, trim::Job>>,
    last_toggle: Mutex<Instant>,
    meta: tokio::sync::Mutex<MetaCache>,
}

#[derive(Default)]
pub struct RecordingState {
    service: tokio::sync::OnceCell<Arc<Service>>,
}

impl RecordingState {
    /// Detiene una grabación en curso al cerrar la aplicación, para que el archivo quede completo.
    pub fn close(&self) {
        if let Some(service) = self.service.get() {
            let service = service.clone();
            tauri::async_runtime::block_on(async move {
                let _ = tokio::time::timeout(Duration::from_secs(40), service.stop_recording("Strata se cerró")).await;
                service.hotkeys.unregister();
            });
        }
    }
}

async fn service(app: &tauri::AppHandle, state: &RecordingState) -> Result<Arc<Service>, String> {
    state
        .service
        .get_or_try_init(|| async {
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("recording");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let settings = settings::load(&dir);
            let (press_tx, mut press_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
            let hotkeys = Hotkeys::start(move || {
                let _ = press_tx.send(());
            })?;
            let overlay = Overlay::start()?;
            let meta = MetaCache::load(&dir.join("meta.json"));
            // Las miniaturas se sirven por el protocolo de assets: se permite solo su carpeta. Los vídeos se permiten de uno
            // en uno al reproducirlos (`recording_allow_playback`), no carpetas enteras.
            let _ = app.asset_protocol_scope().allow_directory(dir.join("thumbs"), true);
            let service = Arc::new(Service {
                app: app.clone(),
                dir,
                settings: Mutex::new(settings),
                encoders: tokio::sync::Mutex::new(None),
                probing: AtomicBool::new(false),
                installing: AtomicBool::new(false),
                recording: AtomicBool::new(false),
                active: tokio::sync::Mutex::new(None),
                games: Mutex::new(Vec::new()),
                hotkey_status: Mutex::new(HotkeyStatus::default()),
                registered_for: Mutex::new(None),
                hotkeys,
                overlay,
                trims: Mutex::new(HashMap::new()),
                last_toggle: Mutex::new(Instant::now() - Duration::from_secs(10)),
                meta: tokio::sync::Mutex::new(meta),
            });
            let pressing = service.clone();
            tokio::spawn(async move {
                while press_rx.recv().await.is_some() {
                    pressing.toggle().await;
                }
            });
            let watching = service.clone();
            tokio::spawn(async move { watching.watch_games().await });
            Ok::<_, String>(service)
        })
        .await
        .cloned()
}

/// Arranca el servicio al abrir Strata: así la tecla funciona aunque no se abra nunca la pantalla de capturas.
pub fn start_in_background(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<RecordingState>();
        if let Err(error) = service(&app, &state).await {
            tracing::warn!("No se pudo iniciar el servicio de grabación: {error}");
        }
    });
}

impl Service {
    pub(super) fn emit(&self, payload: serde_json::Value) {
        let _ = self.app.emit("recording-event", payload);
    }

    pub(super) fn settings(&self) -> RecordingSettings {
        self.settings.lock().unwrap().clone()
    }

    // ------------------------------------------------------------ juego y tecla

    /// Cada segundo y medio mira qué Minecraft lanzados desde Strata hay en marcha y registra o libera la tecla.
    async fn watch_games(self: Arc<Self>) {
        let mut previous: Vec<u32> = Vec::new();
        loop {
            if let Ok(processes) = theseus::process::get_all().await {
                let games: Vec<Game> = processes
                    .into_iter()
                    .filter_map(|p| p.pid.map(|pid| Game { pid, instance_id: p.instance_id, instance_name: p.instance_name }))
                    .collect();
                let pids: Vec<u32> = games.iter().map(|g| g.pid).collect();
                *self.games.lock().unwrap() = games;
                if pids != previous {
                    self.emit(serde_json::json!({ "type": "game", "running": !pids.is_empty() }));
                    previous = pids;
                }
            }
            self.sync_hotkey();
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }

    /// Registra la tecla solo si la grabación está activada y hay un juego en marcha (o se está grabando).
    pub(super) fn sync_hotkey(&self) {
        let settings = self.settings();
        let game_running = !self.games.lock().unwrap().is_empty();
        // Aunque se desactive la grabación mientras se graba, la tecla sigue registrada hasta detenerla.
        let wanted = ((settings.enabled || self.recording.load(Ordering::Relaxed)) && game_running).then(|| settings.hotkey.clone());
        let mut registered = self.registered_for.lock().unwrap();
        if *registered == wanted {
            return;
        }
        let mut status = self.hotkey_status.lock().unwrap();
        match &wanted {
            Some(hotkey) => match self.hotkeys.register(hotkey) {
                Ok(()) => {
                    *registered = wanted.clone();
                    *status = HotkeyStatus { label: hotkey.label(), registered: true, error: None };
                }
                Err(error) => {
                    // No se reintenta cada segundo: se vuelve a intentar al cambiar la tecla o al volver a lanzar el juego.
                    *registered = wanted.clone();
                    *status = HotkeyStatus { label: hotkey.label(), registered: false, error: Some(error) };
                }
            },
            None => {
                self.hotkeys.unregister();
                *registered = None;
                *status = HotkeyStatus { label: settings.hotkey.label(), registered: false, error: None };
            }
        }
        self.emit(serde_json::json!({ "type": "hotkey", "registered": status.registered, "error": status.error }));
    }

    /// Pulsación de la tecla: empieza o termina la grabación. Ignora pulsaciones repetidas en menos de 0,8 s.
    pub(super) async fn toggle(self: &Arc<Self>) {
        {
            let mut last = self.last_toggle.lock().unwrap();
            if last.elapsed() < Duration::from_millis(800) {
                return;
            }
            *last = Instant::now();
        }
        let recording = self.active.lock().await.is_some();
        let outcome = if recording {
            self.stop_recording("Detenida con la tecla").await.map(|_| ())
        } else {
            self.start_recording().await
        };
        if let Err(message) = outcome {
            self.emit(serde_json::json!({ "type": "error", "message": message }));
        }
    }

    // ------------------------------------------------------------ codificadores

    pub(super) async fn ensure_encoders(&self) -> Result<Vec<EncoderStatus>, String> {
        let mut guard = self.encoders.lock().await;
        if let Some(found) = guard.as_ref() {
            return Ok(found.clone());
        }
        if !ffmpeg::is_installed(&self.dir) {
            return Err("Falta el componente de grabación (FFmpeg)".into());
        }
        self.probing.store(true, Ordering::Relaxed);
        let found = ffmpeg::probe_encoders(&ffmpeg::paths(&self.dir).ffmpeg).await;
        self.probing.store(false, Ordering::Relaxed);
        *guard = Some(found.clone());
        Ok(found)
    }

    pub(super) async fn pick_encoder(&self, mode: &str) -> Result<&'static str, String> {
        let found = self.ensure_encoders().await?;
        ffmpeg::choose_encoder(mode, &found).ok_or_else(|| match mode {
            "hardware" => "Tu equipo no tiene un codificador por hardware compatible. Elige «Automático» o «Software».".to_string(),
            _ => "No hay ningún codificador de vídeo que funcione en este equipo.".to_string(),
        })
    }

    // ------------------------------------------------------------ biblioteca

    /// (id, nombre, carpeta completa) de cada instancia.
    pub(super) async fn instance_dirs(&self) -> Vec<(String, String, PathBuf)> {
        let mut found = Vec::new();
        if let Ok(instances) = theseus::instance::list().await {
            for metadata in instances {
                let instance = metadata.instance;
                if let Ok(path) = theseus::instance::get_full_path(&instance.id).await {
                    found.push((instance.id.clone(), instance.name.clone(), path));
                }
            }
        }
        found
    }

    pub(super) async fn sources(&self) -> Vec<Source> {
        let custom = self.settings().custom_folder();
        library::sources(&self.instance_dirs().await, custom.as_deref())
    }

    pub(super) async fn managed_roots(&self) -> Vec<PathBuf> {
        self.sources().await.into_iter().map(|s| s.dir).collect()
    }

    async fn list_videos(self: &Arc<Self>) -> Vec<VideoItem> {
        let sources = self.sources().await;
        let installed = ffmpeg::is_installed(&self.dir);
        let ffprobe = ffmpeg::paths(&self.dir).ffprobe;
        let mut items = Vec::new();
        let mut to_probe: Vec<(usize, PathBuf)> = Vec::new();
        let mut keep = std::collections::HashSet::new();
        {
            let cache = self.meta.lock().await;
            for source in &sources {
                for (path, meta) in library::scan(&source.dir) {
                    let id = library::video_id(&path);
                    keep.insert(id.clone());
                    let info = cache.get(&id, &meta);
                    if info.is_none() && installed {
                        to_probe.push((items.len(), path.clone()));
                    }
                    items.push(library::to_item(source, &path, &meta, info.as_ref()));
                }
            }
        }
        // Se leen con ffprobe (3 a la vez) solo los vídeos que aún no están en la caché.
        if !to_probe.is_empty() {
            let limit = Arc::new(tokio::sync::Semaphore::new(3));
            let mut tasks = tokio::task::JoinSet::new();
            for (index, path) in to_probe {
                let (limit, ffprobe) = (limit.clone(), ffprobe.clone());
                tasks.spawn(async move {
                    let _permit = limit.acquire_owned().await;
                    (index, ffmpeg::probe_media(&ffprobe, &path).await.ok())
                });
            }
            let mut cache = self.meta.lock().await;
            while let Some(Ok((index, info))) = tasks.join_next().await {
                let Some(info) = info else { continue };
                let item = &mut items[index];
                item.duration = info.duration;
                item.width = info.width;
                item.height = info.height;
                item.has_audio = Some(info.has_audio);
                if let Ok(meta) = std::fs::metadata(&item.path) {
                    cache.put(&item.id, &meta, info);
                }
            }
            cache.save(&self.dir.join("meta.json"));
        }
        if installed {
            let mut cache = self.meta.lock().await;
            cache.retain(&keep);
        }
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        items
    }

    /// Grabaciones interrumpidas (cierre inesperado del juego o del launcher): se convierten en vídeos normales.
    async fn recover_partials(self: &Arc<Self>) {
        if !ffmpeg::is_installed(&self.dir) {
            return;
        }
        let active_partial = self.active.lock().await.as_ref().map(|a| a.partial.clone());
        let ffmpeg_path = ffmpeg::paths(&self.dir).ffmpeg;
        for source in self.sources().await {
            for partial in library::partials(&source.dir) {
                if active_partial.as_ref() == Some(&partial) {
                    continue;
                }
                // Si se modificó hace muy poco, puede que otra instancia de Strata la esté escribiendo.
                let recent = std::fs::metadata(&partial).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e < Duration::from_secs(20));
                if recent {
                    continue;
                }
                let name = partial.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let stem = name.trim_end_matches(&format!(".{}", library::PARTIAL_EXTENSION)).to_string();
                let format = if stem.ends_with(".mkv") { "mkv" } else { "mp4" };
                let base = Path::new(&stem).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "grabación".into());
                let output = library::unique_path(&source.dir, &format!("{base} (recuperada)"), format);
                let audio_partial = library::audio_partial_path(&partial);
                let audio = std::fs::metadata(&audio_partial).is_ok_and(|m| m.len() > 4096).then_some(audio_partial.as_path());
                let ok = capture::finalize_file(&ffmpeg_path, &partial, audio, &output, format).await;
                let len = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
                if ok && len > 4096 {
                    let _ = std::fs::remove_file(&partial);
                    let _ = std::fs::remove_file(&audio_partial);
                } else {
                    let _ = std::fs::remove_file(&output);
                    // Sin imagen que recuperar: no se deja basura.
                    if std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0) < 4096 {
                        let _ = std::fs::remove_file(&partial);
                        let _ = std::fs::remove_file(&audio_partial);
                    }
                }
            }
        }
    }

    pub(super) fn thumbs_dir(&self) -> PathBuf {
        self.dir.join("thumbs")
    }

    pub(super) async fn forget_video(&self, path: &Path) {
        let id = library::video_id(path);
        self.meta.lock().await.forget(&id);
        if let Ok(entries) = std::fs::read_dir(self.thumbs_dir()) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&format!("{id}-")) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    async fn status(&self) -> Status {
        let settings = self.settings();
        let encoders = self.encoders.lock().await.clone();
        let encoder_in_use = encoders.as_ref().and_then(|e| ffmpeg::choose_encoder(&settings.encoder, e));
        let recording = self.active.lock().await.as_ref().map(|a| RecordingInfo {
            instance_name: a.instance_name.clone(),
            elapsed_seconds: a.started.elapsed().as_secs(),
            encoder: a.encoder.clone(),
            path: a.final_path.to_string_lossy().into_owned(),
        });
        let mut hotkey = self.hotkey_status.lock().unwrap().clone();
        if hotkey.label.is_empty() {
            hotkey.label = settings.hotkey.label();
        }
        Status {
            supported: true,
            settings,
            ffmpeg: FfmpegStatus {
                installed: ffmpeg::is_installed(&self.dir),
                version: ffmpeg::VERSION,
                installing: self.installing.load(Ordering::Relaxed),
                download_mb: ffmpeg::DOWNLOAD_SIZE / 1_048_576,
            },
            encoders,
            encoder_in_use,
            probing_encoders: self.probing.load(Ordering::Relaxed),
            recording,
            hotkey,
            game_running: !self.games.lock().unwrap().is_empty(),
        }
    }
}

use super::capture;

// ------------------------------------------------------------------ comandos

#[tauri::command]
pub async fn recording_support() -> bool {
    true
}

#[tauri::command]
pub async fn recording_status(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>) -> Result<Status, String> {
    let service = service(&app, &state).await?;
    // Si FFmpeg ya está y aún no se han probado los codificadores, se hace en segundo plano (puede tardar unos segundos).
    if ffmpeg::is_installed(&service.dir) && !service.probing.load(Ordering::Relaxed) && service.encoders.lock().await.is_none() {
        let probing = service.clone();
        tokio::spawn(async move {
            // `ensure_encoders` marca el estado «probando» mientras trabaja; una segunda llamada espera su resultado.
            let _ = probing.ensure_encoders().await;
            probing.emit(serde_json::json!({ "type": "encoders" }));
        });
    }
    Ok(service.status().await)
}

#[tauri::command]
pub async fn recording_set_settings(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>, settings: RecordingSettings) -> Result<Status, String> {
    let service = service(&app, &state).await?;
    settings.validate()?;
    settings::save(&service.dir, &settings)?;
    if let Some(folder) = settings.custom_folder() {
        std::fs::create_dir_all(&folder).map_err(|e| format!("No se pudo crear la carpeta de destino: {e}"))?;
    }
    *service.settings.lock().unwrap() = settings;
    service.sync_hotkey();
    Ok(service.status().await)
}

#[tauri::command]
pub async fn recording_install_ffmpeg(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>) -> Result<Status, String> {
    let service = service(&app, &state).await?;
    if service.installing.swap(true, Ordering::SeqCst) {
        return Err("La descarga ya está en curso".into());
    }
    let emitter = service.clone();
    let result = ffmpeg::install(&service.dir, move |progress| {
        emitter.emit(serde_json::json!({ "type": "ffmpeg", "phase": progress.phase, "downloaded": progress.downloaded, "total": progress.total }));
    })
    .await;
    service.installing.store(false, Ordering::SeqCst);
    result?;
    *service.encoders.lock().await = None;
    let _ = service.ensure_encoders().await;
    service.emit(serde_json::json!({ "type": "encoders" }));
    Ok(service.status().await)
}

#[tauri::command]
pub async fn recording_remove_ffmpeg(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>) -> Result<Status, String> {
    let service = service(&app, &state).await?;
    if service.active.lock().await.is_some() {
        return Err("Detén la grabación antes de quitar el componente".into());
    }
    if !service.trims.lock().unwrap().is_empty() {
        return Err("Hay un recorte en curso".into());
    }
    let dir = ffmpeg::paths(&service.dir).dir;
    if dir.starts_with(service.dir.join("bin")) {
        let _ = std::fs::remove_dir_all(&dir);
    }
    *service.encoders.lock().await = None;
    Ok(service.status().await)
}

/// Vuelve a probar los codificadores (por ejemplo tras actualizar el controlador de la tarjeta gráfica).
#[tauri::command]
pub async fn recording_reprobe_encoders(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>) -> Result<Status, String> {
    let service = service(&app, &state).await?;
    *service.encoders.lock().await = None;
    service.ensure_encoders().await?;
    Ok(service.status().await)
}

#[derive(Serialize)]
pub struct Conflict {
    pub source: String,
    pub binding: String,
}

/// Atajos de Minecraft que ya usan la tecla elegida (los predeterminados y los de `options.txt` de cada instancia).
#[tauri::command]
pub async fn recording_check_conflicts(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>, hotkey: Hotkey) -> Result<Vec<Conflict>, String> {
    let service = service(&app, &state).await?;
    hotkey.validate()?;
    let mut conflicts = Vec::new();
    if let Some(binding) = settings::minecraft_default_binding(&hotkey.key) {
        conflicts.push(Conflict { source: "Minecraft".into(), binding: binding.to_string() });
    }
    if let Some(key_name) = settings::minecraft_key_name(&hotkey.key) {
        for (_, name, path) in service.instance_dirs().await {
            let Ok(text) = std::fs::read_to_string(path.join("options.txt")) else { continue };
            for binding in settings::scan_options_conflicts(&text, &key_name) {
                conflicts.push(Conflict { source: name.clone(), binding });
            }
        }
    }
    Ok(conflicts)
}

/// Empieza o detiene la grabación del juego en marcha (lo mismo que hace la tecla).
#[tauri::command]
pub async fn recording_toggle(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>) -> Result<(), String> {
    let service = service(&app, &state).await?;
    if service.active.lock().await.is_some() {
        service.stop_recording("Detenida desde Strata").await.map(|_| ())
    } else {
        service.start_recording().await
    }
}

#[tauri::command]
pub async fn recording_list(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>) -> Result<Vec<VideoItem>, String> {
    let service = service(&app, &state).await?;
    let recovering = service.clone();
    tokio::spawn(async move {
        recovering.recover_partials().await;
    });
    Ok(service.list_videos().await)
}

#[tauri::command]
pub async fn recording_thumbnail(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>, path: String) -> Result<Option<String>, String> {
    let service = service(&app, &state).await?;
    let path = PathBuf::from(path);
    if !library::is_managed(&path, &service.managed_roots().await) {
        return Err("Ese vídeo no está en una carpeta de grabaciones".into());
    }
    if !ffmpeg::is_installed(&service.dir) {
        return Ok(None);
    }
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let thumbs = service.thumbs_dir();
    std::fs::create_dir_all(&thumbs).map_err(|e| e.to_string())?;
    let output = thumbs.join(format!("{}-{stamp}.jpg", library::video_id(&path)));
    if output.exists() {
        return Ok(Some(output.to_string_lossy().into_owned()));
    }
    let paths = ffmpeg::paths(&service.dir);
    let duration = ffmpeg::probe_media(&paths.ffprobe, &path).await.ok().and_then(|i| i.duration).unwrap_or(0.0);
    let at = (duration * 0.1).clamp(0.0, 2.0);
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args(super::args::thumbnail_args(&path, &output, at))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let status = tokio::time::timeout(Duration::from_secs(30), cmd.status()).await.map_err(|_| "La miniatura tarda demasiado".to_string())?.map_err(|e| e.to_string())?;
    if status.success() && output.exists() {
        Ok(Some(output.to_string_lossy().into_owned()))
    } else {
        let _ = std::fs::remove_file(&output);
        Ok(None)
    }
}

#[tauri::command]
pub async fn recording_delete(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>, path: String) -> Result<(), String> {
    let service = service(&app, &state).await?;
    let path = PathBuf::from(path);
    if !library::is_managed(&path, &service.managed_roots().await) {
        return Err("Ese vídeo no está en una carpeta de grabaciones".into());
    }
    if service.active.lock().await.as_ref().is_some_and(|a| a.final_path == path) {
        return Err("Ese vídeo se está grabando ahora".into());
    }
    std::fs::remove_file(&path).map_err(|e| format!("No se pudo eliminar el vídeo: {e}"))?;
    service.forget_video(&path).await;
    Ok(())
}

#[tauri::command]
pub async fn recording_reveal(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>, path: String) -> Result<(), String> {
    let service = service(&app, &state).await?;
    let path = PathBuf::from(path);
    if !library::is_managed(&path, &service.managed_roots().await) {
        return Err("Ese vídeo no está en una carpeta de grabaciones".into());
    }
    app.opener().reveal_item_in_dir(&path).map_err(|e| e.to_string())
}

/// Permite reproducir un vídeo por el protocolo de assets (solo los de las carpetas de grabaciones).
#[tauri::command]
pub async fn recording_allow_playback(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>, path: String) -> Result<(), String> {
    let service = service(&app, &state).await?;
    let path = PathBuf::from(path);
    if !library::is_managed(&path, &service.managed_roots().await) {
        return Err("Ese vídeo no está en una carpeta de grabaciones".into());
    }
    app.asset_protocol_scope().allow_file(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recording_trim_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, RecordingState>,
    path: String,
    start: f64,
    end: f64,
    precise: bool,
    gif: Option<bool>,
) -> Result<String, String> {
    let service = service(&app, &state).await?;
    service.trim_start(PathBuf::from(path), start, end, precise, gif.unwrap_or(false)).await
}

#[tauri::command]
pub async fn recording_trim_cancel(app: tauri::AppHandle, state: tauri::State<'_, RecordingState>, job: String) -> Result<(), String> {
    let service = service(&app, &state).await?;
    service.trim_cancel(&job);
    Ok(())
}
