//! Grabador: lanza FFmpeg con la captura de la ventana de Minecraft (`gfxcapture`), conecta el audio del juego y,
//! al parar, cierra el archivo con cuidado para que quede reproducible.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::args::{self, AudioArgs, CaptureSpec, VideoArgs};
use super::audio;
use super::ffmpeg;
use super::library::{self, PARTIAL_EXTENSION, VideoItem};
use super::service::{Game, Service};
use super::win;

/// Microsegundos desde 1970 (el mismo reloj que `RTCTIME` de FFmpeg).
fn now_micros() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// Espacio libre mínimo para empezar a grabar y para seguir haciéndolo.
const MIN_FREE_TO_START: u64 = 1024 * 1024 * 1024;
const MIN_FREE_TO_CONTINUE: u64 = 300 * 1024 * 1024;

pub struct Active {
    pub id: String,
    pub started: Instant,
    pub instance_name: String,
    pub partial: PathBuf,
    pub final_path: PathBuf,
    pub format: String,
    pub encoder: String,
    pub hwnd: isize,
    child: tokio::process::Child,
    /// Segundo proceso de FFmpeg, solo audio (ver `args::VideoArgs`). `None` si no se graba audio.
    audio_child: Option<tokio::process::Child>,
    audio_partial: PathBuf,
    stop_audio: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    audio_threads: Vec<std::thread::JoinHandle<()>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    warnings: Vec<String>,
}

/// Convierte lo último que dijo FFmpeg en un mensaje comprensible.
fn explain_failure(tail: &[String]) -> String {
    let text = tail.join(" | ");
    let hint = if text.contains("gfxcapture") && (text.contains("Failed") || text.contains("rror")) {
        "No se pudo capturar la ventana del juego (¿está minimizada o es una ventana protegida?)."
    } else if text.contains("Could not open encoder") || text.contains("Error while opening encoder") {
        "El codificador de vídeo no pudo arrancar. Prueba el modo «Software» en los ajustes."
    } else if text.contains("Permission denied") || text.contains("No such file") {
        "No se pudo escribir en la carpeta de destino."
    } else {
        "FFmpeg se detuvo al empezar a grabar."
    };
    match tail.last() {
        Some(last) if !last.trim().is_empty() => format!("{hint} Detalle: {}", last.trim()),
        _ => hint.to_string(),
    }
}

/// Convierte un archivo (grabación fragmentada o interrumpida) en un vídeo normal sin recodificar.
pub async fn finalize_file(ffmpeg_path: &Path, video: &Path, audio: Option<&Path>, output: &Path, format: &str) -> bool {
    // El destino lleva el sufijo temporal hasta que termina bien: nunca queda un vídeo a medias en la biblioteca.
    // (distinto de `.partial`, que es la extensión de la grabación de entrada).
    let temp = PathBuf::from(format!("{}.finalizing", output.display()));
    let _ = std::fs::remove_file(&temp);
    let mut cmd = ffmpeg::command(ffmpeg_path);
    cmd.args(args::finalize_args(video, audio, &temp, format)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let ok = matches!(tokio::time::timeout(Duration::from_secs(600), cmd.status()).await, Ok(Ok(status)) if status.success());
    if ok && std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0) > 0 && std::fs::rename(&temp, output).is_ok() {
        return true;
    }
    let _ = std::fs::remove_file(&temp);
    false
}

impl Service {
    fn pick_game(&self) -> Result<(win::GameWindow, Game), String> {
        let games = self.games.lock().unwrap().clone();
        if games.is_empty() {
            return Err("No hay ningún Minecraft en marcha lanzado desde Strata".into());
        }
        let pids: Vec<u32> = games.iter().map(|g| g.pid).collect();
        // Con varias instancias abiertas se graba la que tiene el foco.
        let window = win::foreground_pid()
            .filter(|pid| pids.contains(pid))
            .and_then(|pid| win::find_window(&[pid]))
            .or_else(|| win::find_window(&pids))
            .ok_or("No se encontró la ventana de Minecraft (¿todavía está cargando?)")?;
        let game = games.into_iter().find(|g| g.pid == window.pid).ok_or("No se encontró el proceso del juego")?;
        Ok((window, game))
    }

    pub(super) async fn start_recording(self: &Arc<Self>) -> Result<(), String> {
        let mut slot = self.active.lock().await;
        if slot.is_some() {
            return Err("Ya hay una grabación en curso".into());
        }
        let settings = self.settings();
        if !settings.enabled {
            return Err("La grabación está desactivada en los ajustes".into());
        }
        if !ffmpeg::is_installed(&self.dir) {
            self.emit(serde_json::json!({ "type": "needs-ffmpeg" }));
            return Err("Falta el componente de grabación: instálalo en Ajustes → Grabación".into());
        }
        let (window, game) = self.pick_game()?;
        if win::is_minimized(window.hwnd) {
            return Err("La ventana de Minecraft está minimizada".into());
        }
        let (src_w, src_h) = win::client_size(window.hwnd).ok_or("No se pudo medir la ventana del juego")?;
        let (dst_w, dst_h) = args::target_size(src_w, src_h, &settings.resolution);
        let encoder = self.pick_encoder(&settings.encoder).await?;

        let instance_path = theseus::instance::get_full_path(&game.instance_id).await.map_err(|e| e.to_string())?;
        let source = library::sources(&[(game.instance_id.clone(), game.instance_name.clone(), instance_path)], settings.custom_folder().as_deref())
            .into_iter()
            .next()
            .ok_or("No se pudo determinar la carpeta de destino")?;
        std::fs::create_dir_all(&source.dir).map_err(|e| format!("No se pudo crear la carpeta de grabaciones: {e}"))?;
        if let Some(free) = win::free_space(&source.dir) {
            if free < MIN_FREE_TO_START {
                return Err(format!("Queda poco espacio en el disco ({} MB). Libera al menos 1 GB para grabar.", free / 1_048_576));
            }
        }
        let final_path = library::unique_path(&source.dir, &library::timestamp_name(&game.instance_name), &settings.format);
        let partial = PathBuf::from(format!("{}.{}", final_path.display(), PARTIAL_EXTENSION));
        let audio_partial = library::audio_partial_path(&partial);

        // Audio: las tuberías deben existir antes de lanzar FFmpeg, que se conecta a ellas como cliente.
        let id = uuid::Uuid::new_v4().simple().to_string();
        let stop_audio = Arc::new(AtomicBool::new(false));
        let mut tasks = Vec::new();
        let mut audio_threads = Vec::new();
        let mut warnings = Vec::new();
        let mut open_pipe = |label: &str, game_audio: bool, tasks: &mut Vec<tokio::task::JoinHandle<()>>, threads: &mut Vec<std::thread::JoinHandle<()>>| -> Option<String> {
            let name = format!(r"\\.\pipe\strata-rec-{id}-{label}");
            let server = match audio::create_pipe(&name) {
                Ok(server) => server,
                Err(error) => {
                    warnings.push(format!("Sin {label}: {error}"));
                    return None;
                }
            };
            let queue = audio::new_queue();
            let started = if game_audio {
                audio::start_game_capture(window.pid, queue.clone(), stop_audio.clone())
            } else {
                audio::start_microphone_capture(queue.clone(), stop_audio.clone())
            };
            match started {
                Ok(thread) => {
                    threads.push(thread);
                    let stop = stop_audio.clone();
                    tasks.push(tokio::spawn(async move {
                        if let Err(error) = audio::write_pcm(server, queue, stop).await {
                            tracing::warn!("Audio de la grabación: {error}");
                        }
                    }));
                    Some(name)
                }
                Err(error) => {
                    warnings.push(format!("Sin {}: {error}", if game_audio { "audio del juego" } else { "micrófono" }));
                    None
                }
            }
        };
        let game_pipe = if settings.game_audio { open_pipe("juego", true, &mut tasks, &mut audio_threads) } else { None };
        let mic_pipe = if settings.microphone { open_pipe("micro", false, &mut tasks, &mut audio_threads) } else { None };

        let spec = CaptureSpec { hwnd: window.hwnd as i64, fps: settings.fps, src_w, src_h, dst_w, dst_h, clock_origin_us: now_micros() };
        let ffmpeg_path = ffmpeg::paths(&self.dir).ffmpeg;
        let abort = |stop_audio: &Arc<AtomicBool>, tasks: Vec<tokio::task::JoinHandle<()>>| {
            stop_audio.store(true, Ordering::Relaxed);
            for task in tasks {
                task.abort();
            }
        };
        // El audio va en su propio proceso: dentro del de vídeo frenaba la captura a la mitad de fotogramas.
        let mut audio_child = None;
        if let Some(audio_args) =
            args::audio_recording_args(&AudioArgs { clock_origin_us: spec.clock_origin_us, game_pipe: game_pipe.as_deref(), mic_pipe: mic_pipe.as_deref(), output: &audio_partial })
        {
            let mut audio_cmd = ffmpeg::command(&ffmpeg_path);
            audio_cmd.args(audio_args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
            match audio_cmd.spawn() {
                Ok(child) => audio_child = Some(child),
                Err(error) => {
                    warnings.push(format!("Sin audio: {error}"));
                    stop_audio.store(true, Ordering::Relaxed);
                    for task in tasks.drain(..) {
                        task.abort();
                    }
                }
            }
        }
        let discard_audio = |audio_child: Option<tokio::process::Child>, audio_partial: PathBuf| async move {
            if let Some(mut child) = audio_child {
                let _ = child.kill().await;
            }
            let _ = std::fs::remove_file(&audio_partial);
        };
        let mut cmd = ffmpeg::command(&ffmpeg_path);
        cmd.args(args::video_recording_args(&VideoArgs { spec: &spec, encoder, quality: &settings.quality, output: &partial }))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                abort(&stop_audio, tasks);
                discard_audio(audio_child, audio_partial).await;
                return Err(format!("No se pudo iniciar FFmpeg: {error}"));
            }
        };

        // Se lee el progreso de FFmpeg: la primera línea con `frame=` significa que ya está grabando.
        let frames = Arc::new(AtomicU64::new(0));
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let (frames, tail) = (frames.clone(), tail.clone());
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                while let Ok(read) = stderr.read(&mut chunk).await {
                    if read == 0 {
                        break;
                    }
                    for &byte in &chunk[..read] {
                        if byte == b'\r' || byte == b'\n' {
                            if buffer.is_empty() {
                                continue;
                            }
                            let line = String::from_utf8_lossy(&buffer).into_owned();
                            buffer.clear();
                            if let Some(count) = line.split("frame=").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<u64>().ok()) {
                                frames.store(count, Ordering::Relaxed);
                            } else {
                                let mut tail = tail.lock().unwrap();
                                tail.push_back(line);
                                if tail.len() > 30 {
                                    tail.pop_front();
                                }
                            }
                        } else {
                            buffer.push(byte);
                        }
                    }
                }
            });
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let started_ok = loop {
            if frames.load(Ordering::Relaxed) > 0 {
                break true;
            }
            if let Ok(Some(_)) = child.try_wait() {
                break false;
            }
            if Instant::now() > deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        if !started_ok {
            let _ = child.kill().await;
            abort(&stop_audio, tasks);
            discard_audio(audio_child, audio_partial).await;
            let _ = std::fs::remove_file(&partial);
            let lines: Vec<String> = tail.lock().unwrap().iter().cloned().collect();
            return Err(explain_failure(&lines));
        }

        let started = Instant::now();
        let active = Active {
            id: id.clone(),
            started,
            instance_name: game.instance_name.clone(),
            partial,
            final_path,
            format: settings.format.clone(),
            encoder: encoder.to_string(),
            hwnd: window.hwnd,
            child,
            audio_child,
            audio_partial,
            stop_audio,
            tasks,
            audio_threads,
            stderr_tail: tail,
            warnings: warnings.clone(),
        };
        *slot = Some(active);
        self.recording.store(true, Ordering::Relaxed);
        drop(slot);
        if settings.indicator {
            self.overlay.show(window.hwnd, started);
        }
        self.emit(serde_json::json!({
            "type": "started",
            "instance_name": game.instance_name,
            "encoder": encoder,
            "warnings": warnings,
        }));
        let watching = self.clone();
        tokio::spawn(async move { watching.watch_recording(id).await });
        Ok(())
    }

    /// Vigila la grabación: se detiene sola si el juego se cierra, FFmpeg termina o se acaba el espacio.
    async fn watch_recording(self: Arc<Self>, id: String) {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let reason = {
                let mut guard = self.active.lock().await;
                let Some(active) = guard.as_mut() else { return };
                if active.id != id {
                    return;
                }
                if matches!(active.child.try_wait(), Ok(Some(_))) {
                    Some("FFmpeg terminó por su cuenta")
                } else if !win::is_alive(active.hwnd) {
                    Some("Se cerró la ventana del juego")
                } else if win::free_space(&active.final_path).is_some_and(|free| free < MIN_FREE_TO_CONTINUE) {
                    Some("Se agotó el espacio en el disco")
                } else {
                    None
                }
            };
            if let Some(reason) = reason {
                if let Err(message) = self.stop_recording(reason).await {
                    self.emit(serde_json::json!({ "type": "error", "message": message }));
                }
                return;
            }
        }
    }

    /// Termina la grabación y deja un vídeo completo. Devuelve `None` si no había nada que guardar.
    pub(super) async fn stop_recording(self: &Arc<Self>, reason: &str) -> Result<Option<VideoItem>, String> {
        let Some(mut active) = self.active.lock().await.take() else { return Ok(None) };
        self.recording.store(false, Ordering::Relaxed);
        // Si la grabación estaba desactivada en los ajustes, la tecla se libera ahora.
        self.sync_hotkey();
        self.overlay.hide();
        active.stop_audio.store(true, Ordering::Relaxed);
        // `q` pide a FFmpeg que cierre el archivo con normalidad.
        if let Some(mut stdin) = active.child.stdin.take() {
            let _ = stdin.write_all(b"q").await;
            let _ = stdin.flush().await;
        }
        if let Some(audio) = active.audio_child.as_mut() {
            if let Some(mut stdin) = audio.stdin.take() {
                let _ = stdin.write_all(b"q").await;
                let _ = stdin.flush().await;
            }
        }
        if tokio::time::timeout(Duration::from_secs(25), active.child.wait()).await.is_err() {
            let _ = active.child.kill().await;
        }
        if let Some(audio) = active.audio_child.as_mut() {
            if tokio::time::timeout(Duration::from_secs(10), audio.wait()).await.is_err() {
                let _ = audio.kill().await;
            }
        }
        for task in active.tasks.drain(..) {
            let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
        }
        let threads: Vec<_> = active.audio_threads.drain(..).collect();
        let _ = tokio::task::spawn_blocking(move || {
            for thread in threads {
                let _ = thread.join();
            }
        })
        .await;

        let partial_size = std::fs::metadata(&active.partial).map(|m| m.len()).unwrap_or(0);
        if partial_size < 4096 {
            let _ = std::fs::remove_file(&active.partial);
            let _ = std::fs::remove_file(&active.audio_partial);
            let lines: Vec<String> = active.stderr_tail.lock().unwrap().iter().cloned().collect();
            self.emit(serde_json::json!({ "type": "discarded", "reason": "No se llegó a grabar imagen.", "detail": explain_failure(&lines) }));
            return Ok(None);
        }
        let paths = ffmpeg::paths(&self.dir);
        // Se unen vídeo y audio en un archivo normal (índice al principio y duración conocida), sin recodificar.
        let audio = std::fs::metadata(&active.audio_partial).is_ok_and(|m| m.len() > 4096).then_some(active.audio_partial.as_path());
        let remuxed = finalize_file(&paths.ffmpeg, &active.partial, audio, &active.final_path, &active.format).await;
        if remuxed {
            let _ = std::fs::remove_file(&active.partial);
            let _ = std::fs::remove_file(&active.audio_partial);
        } else {
            // Los temporales se conservan: al abrir Strata se reintenta y se recuperan como «(recuperada)».
            return Err("No se pudo guardar el vídeo. Los temporales se conservan y se recuperarán al abrir Strata.".into());
        }
        let info = ffmpeg::probe_media(&paths.ffprobe, &active.final_path).await.ok();
        if info.as_ref().and_then(|i| i.duration).is_some_and(|d| d < 0.5) {
            let _ = std::fs::remove_file(&active.final_path);
            self.emit(serde_json::json!({ "type": "discarded", "reason": "La grabación duró menos de medio segundo." }));
            return Ok(None);
        }
        let meta = std::fs::metadata(&active.final_path).map_err(|e| e.to_string())?;
        let source = library::Source { instance_id: String::new(), instance_name: active.instance_name.clone(), dir: active.final_path.parent().map(Path::to_path_buf).unwrap_or_default() };
        let mut item = library::to_item(&source, &active.final_path, &meta, info.as_ref());
        if let Some(game) = self.games.lock().unwrap().iter().find(|g| g.instance_name == active.instance_name) {
            item.instance_id = game.instance_id.clone();
        }
        self.emit(serde_json::json!({
            "type": "saved",
            "video": item,
            "reason": reason,
            "warnings": active.warnings,
            "repaired": !remuxed,
        }));
        Ok(Some(item))
    }
}
