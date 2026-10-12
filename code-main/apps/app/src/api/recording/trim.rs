//! Recorte de vídeos: genera un archivo nuevo con el fragmento elegido y deja el original intacto.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use super::args::{self, TrimMode, TrimSource};
use super::ffmpeg;
use super::library;
use super::service::Service;

pub struct Job {
    cancel: Arc<AtomicBool>,
}

pub const MIN_CLIP_SECONDS: f64 = 0.5;
/// Un GIF más largo pesa demasiado (decenas de MB) y casi nadie puede enviarlo.
pub const MAX_GIF_SECONDS: f64 = 30.0;
/// Tope de vídeos por unión (cada uno añade una entrada y un filtro a FFmpeg).
pub const MAX_MERGE_VIDEOS: usize = 8;

/// Comprueba el intervalo elegido y devuelve el final ajustado a la duración real del vídeo.
pub fn validate_range(start: f64, end: f64, duration: f64) -> Result<f64, String> {
    if !start.is_finite() || !end.is_finite() || !duration.is_finite() {
        return Err("El intervalo no es válido".into());
    }
    if start < 0.0 {
        return Err("El inicio no puede ser negativo".into());
    }
    if start >= duration {
        return Err("El inicio está después del final del vídeo".into());
    }
    let end = end.min(duration);
    if end - start < MIN_CLIP_SECONDS {
        return Err("El fragmento debe durar al menos medio segundo".into());
    }
    Ok(end)
}

/// Nombre del recorte: `original (recorte).mp4`, `original (recorte 2).mp4`… Nunca existe ya.
pub fn clip_path(original: &Path) -> PathBuf {
    let dir = original.parent().unwrap_or_else(|| Path::new("."));
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "vídeo".into());
    let ext = original.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "mp4".into());
    let first = dir.join(format!("{stem} (recorte).{ext}"));
    if !first.exists() && !PathBuf::from(format!("{}.trimming", first.display())).exists() {
        return first;
    }
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} (recorte {n}).{ext}"));
        if !candidate.exists() && !PathBuf::from(format!("{}.trimming", candidate.display())).exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} (recorte {}).{ext}", uuid::Uuid::new_v4().simple()))
}

/// Nombre del GIF: `original (gif).gif`, `original (gif 2).gif`… Nunca existe ya.
pub fn gif_path(original: &Path) -> PathBuf {
    let dir = original.parent().unwrap_or_else(|| Path::new("."));
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "vídeo".into());
    let free = |p: &PathBuf| !p.exists() && !PathBuf::from(format!("{}.trimming", p.display())).exists();
    let first = dir.join(format!("{stem} (gif).gif"));
    if free(&first) {
        return first;
    }
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} (gif {n}).gif"));
        if free(&candidate) {
            return candidate;
        }
    }
    dir.join(format!("{stem} (gif {}).gif", uuid::Uuid::new_v4().simple()))
}

/// Nombre de la unión: `primero (unido).mp4`, `primero (unido 2).mp4`… Nunca existe ya.
pub fn merged_path(first: &Path, format: &str) -> PathBuf {
    let dir = first.parent().unwrap_or_else(|| Path::new("."));
    let stem = first.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "vídeo".into());
    let free = |p: &PathBuf| !p.exists() && !PathBuf::from(format!("{}.trimming", p.display())).exists();
    let candidate = dir.join(format!("{stem} (unido).{format}"));
    if free(&candidate) {
        return candidate;
    }
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} (unido {n}).{format}"));
        if free(&candidate) {
            return candidate;
        }
    }
    dir.join(format!("{stem} (unido {}).{format}", uuid::Uuid::new_v4().simple()))
}

/// Segundos de la línea `out_time_us=…` / `out_time_ms=…` de `-progress`.
pub fn parse_progress_line(line: &str) -> Option<f64> {
    let value = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms="))?;
    value.trim().parse::<f64>().ok().filter(|v| *v >= 0.0).map(|micros| micros / 1_000_000.0)
}

impl Service {
    pub(super) fn trim_cancel(&self, job: &str) {
        if let Some(job) = self.trims.lock().unwrap().get(job) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) async fn trim_start(self: &Arc<Self>, path: PathBuf, start: f64, end: f64, precise: bool, gif: bool, speed: f64, mute: bool) -> Result<String, String> {
        if !args::TRIM_SPEEDS.contains(&speed) {
            return Err("Velocidad no válida".into());
        }
        if !library::is_managed(&path, &self.managed_roots().await) {
            return Err("Ese vídeo no está en una carpeta de grabaciones".into());
        }
        if !ffmpeg::is_installed(&self.dir) {
            return Err("Falta el componente de grabación (FFmpeg): instálalo en Ajustes → Grabación".into());
        }
        if !self.trims.lock().unwrap().is_empty() {
            return Err("Ya hay un recorte en curso: espera a que termine".into());
        }
        let paths = ffmpeg::paths(&self.dir);
        let info = ffmpeg::probe_media(&paths.ffprobe, &path).await?;
        let duration = info.duration.ok_or("No se pudo leer la duración del vídeo")?;
        let end = validate_range(start, end, duration)?;
        if gif && end - start > MAX_GIF_SECONDS {
            return Err(format!("Un GIF puede durar como máximo {} segundos: acorta el fragmento", MAX_GIF_SECONDS as u32));
        }

        let settings = self.settings();
        let format = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("mkv")) { "mkv" } else { "mp4" };
        let changes_speed = (speed - 1.0).abs() > f64::EPSILON;
        // Cambiar la velocidad obliga a recodificar: se ignora «rápido».
        let mode = if precise || changes_speed { TrimMode::Precise } else { TrimMode::Fast };
        let encoder = if mode == TrimMode::Precise && !gif { self.pick_encoder(&settings.encoder).await? } else { "libx264" };
        let source = TrimSource {
            encoder,
            quality: &settings.quality,
            size: (info.width.unwrap_or(1280) & !1, info.height.unwrap_or(720) & !1),
            fps: info.fps.map_or(30, |f| f.round() as u32).clamp(1, 120),
            format,
            speed,
            mute,
        };

        let output = if gif { gif_path(&path) } else { clip_path(&path) };
        let temp = PathBuf::from(format!("{}.trimming", output.display()));
        let arguments = if gif { args::gif_args(&path, &temp, start, end) } else { args::trim_args(&path, &temp, start, end, mode, &source) };
        let output_seconds = if gif { end - start } else { (end - start) / speed };
        self.launch_export(arguments, temp, output, output_seconds).await
    }

    /// Lanza FFmpeg con los argumentos dados y avisa del progreso (`wanted` = segundos de salida esperados). El archivo se
    /// escribe como `temp` y solo pasa a `output` si termina bien: nunca queda uno a medias.
    async fn launch_export(self: &Arc<Self>, arguments: Vec<String>, temp: PathBuf, output: PathBuf, wanted: f64) -> Result<String, String> {
        let paths = ffmpeg::paths(&self.dir);
        let mut cmd = ffmpeg::command(&paths.ffmpeg);
        cmd.args(arguments).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("No se pudo iniciar FFmpeg: {e}"))?;

        let job_id = uuid::Uuid::new_v4().simple().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        self.trims.lock().unwrap().insert(job_id.clone(), Job { cancel: cancel.clone() });

        let service = self.clone();
        let id = job_id.clone();
        tokio::spawn(async move {
            let stdout = child.stdout.take();
            let mut stderr = child.stderr.take();
            let errors = tokio::spawn(async move {
                let mut text = String::new();
                if let Some(stderr) = stderr.as_mut() {
                    let _ = stderr.take(8192).read_to_string(&mut text).await;
                }
                text
            });
            let progress = {
                let (service, id) = (service.clone(), id.clone());
                tokio::spawn(async move {
                    let Some(stdout) = stdout else { return };
                    let mut lines = BufReader::new(stdout).lines();
                    let mut last = Instant::now() - Duration::from_secs(1);
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some(seconds) = parse_progress_line(&line) {
                            if last.elapsed() >= Duration::from_millis(250) {
                                last = Instant::now();
                                let fraction = (seconds / wanted).clamp(0.0, 1.0);
                                service.emit(serde_json::json!({ "type": "trim", "job": id, "state": "progress", "progress": fraction }));
                            }
                        }
                    }
                })
            };
            // Se espera a que termine vigilando la cancelación.
            let status = loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = child.kill().await;
                    break None;
                }
                match child.try_wait() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) => tokio::time::sleep(Duration::from_millis(100)).await,
                    Err(_) => break None,
                }
            };
            let _ = progress.await;
            let stderr_text = errors.await.unwrap_or_default();
            service.trims.lock().unwrap().remove(&id);
            let canceled = cancel.load(Ordering::Relaxed);
            let size = std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
            match status {
                Some(status) if status.success() && size > 0 && !canceled => {
                    if std::fs::rename(&temp, &output).is_err() {
                        let _ = std::fs::remove_file(&temp);
                        service.emit(serde_json::json!({ "type": "trim", "job": id, "state": "error", "message": "No se pudo guardar el recorte." }));
                        return;
                    }
                    let result = ffmpeg::probe_media(&ffmpeg::paths(&service.dir).ffprobe, &output).await.ok();
                    service.emit(serde_json::json!({
                        "type": "trim", "job": id, "state": "done",
                        "output": output.to_string_lossy(),
                        "duration": result.and_then(|i| i.duration),
                    }));
                }
                _ => {
                    let _ = std::fs::remove_file(&temp);
                    if canceled {
                        service.emit(serde_json::json!({ "type": "trim", "job": id, "state": "canceled" }));
                    } else {
                        let detail = stderr_text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                        service.emit(serde_json::json!({
                            "type": "trim", "job": id, "state": "error",
                            "message": format!("No se pudo exportar el recorte.{}", if detail.is_empty() { String::new() } else { format!(" Detalle: {detail}") }),
                        }));
                    }
                }
            }
        });
        Ok(job_id)
    }

    /// Une varios vídeos (en el orden dado) en uno nuevo con la resolución y los FPS del primero. Recodifica, así que
    /// admite vídeos de distinto tamaño; los que no tienen sonido se rellenan con silencio.
    pub(super) async fn merge_start(self: &Arc<Self>, paths_in: Vec<PathBuf>) -> Result<String, String> {
        if paths_in.len() < 2 || paths_in.len() > MAX_MERGE_VIDEOS {
            return Err(format!("Elige entre 2 y {MAX_MERGE_VIDEOS} vídeos para unir"));
        }
        let roots = self.managed_roots().await;
        let mut seen = std::collections::HashSet::new();
        for path in &paths_in {
            if !library::is_managed(path, &roots) {
                return Err("Uno de los vídeos no está en una carpeta de grabaciones".into());
            }
            if !seen.insert(path.clone()) {
                return Err("No se puede unir el mismo vídeo dos veces".into());
            }
        }
        if !ffmpeg::is_installed(&self.dir) {
            return Err("Falta el componente de grabación (FFmpeg): instálalo en Ajustes → Grabación".into());
        }
        if !self.trims.lock().unwrap().is_empty() {
            return Err("Ya hay una exportación en curso: espera a que termine".into());
        }
        let ff = ffmpeg::paths(&self.dir);
        let mut infos = Vec::new();
        for path in &paths_in {
            let info = ffmpeg::probe_media(&ff.ffprobe, path).await?;
            let duration = info.duration.filter(|d| *d > 0.0).ok_or("No se pudo leer la duración de uno de los vídeos")?;
            infos.push((duration, info));
        }
        let total: f64 = infos.iter().map(|(d, _)| *d).sum();
        let first = &infos[0].1;
        let size = (first.width.unwrap_or(1280) & !1, first.height.unwrap_or(720) & !1);
        let fps = first.fps.map_or(30, |f| f.round() as u32).clamp(1, 120);
        let settings = self.settings();
        let encoder = self.pick_encoder(&settings.encoder).await?;
        let format = if paths_in[0].extension().is_some_and(|e| e.eq_ignore_ascii_case("mkv")) { "mkv" } else { "mp4" };
        let output = merged_path(&paths_in[0], format);
        let temp = PathBuf::from(format!("{}.trimming", output.display()));
        let inputs: Vec<args::MergeInput> = paths_in
            .iter()
            .zip(&infos)
            .map(|(path, (duration, info))| args::MergeInput { path, duration: *duration, has_audio: info.has_audio })
            .collect();
        let arguments = args::merge_args(&inputs, &temp, size, fps, encoder, &settings.quality, format);
        self.launch_export(arguments, temp, output, total).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_names_never_collide_with_existing_files_or_pending_exports() {
        let dir = std::env::temp_dir().join(format!("strata-gif-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("partida.mp4");
        assert_eq!(gif_path(&video), dir.join("partida (gif).gif"));
        std::fs::write(dir.join("partida (gif).gif"), b"x").unwrap();
        assert_eq!(gif_path(&video), dir.join("partida (gif 2).gif"));
        std::fs::write(dir.join("partida (gif 2).gif.trimming"), b"x").unwrap();
        assert_eq!(gif_path(&video), dir.join("partida (gif 3).gif"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_selected_range_is_validated() {
        assert_eq!(validate_range(35.0, 58.0, 120.0), Ok(58.0));
        assert_eq!(validate_range(10.0, 500.0, 120.0), Ok(120.0), "el final se ajusta a la duración");
        assert!(validate_range(-1.0, 5.0, 120.0).is_err());
        assert!(validate_range(60.0, 60.2, 120.0).is_err(), "demasiado corto");
        assert!(validate_range(130.0, 140.0, 120.0).is_err(), "empieza después del final");
        assert!(validate_range(f64::NAN, 5.0, 120.0).is_err());
        assert!(validate_range(0.0, f64::INFINITY, f64::INFINITY).is_err());
        // Ejemplo del enunciado: de 120 s se conserva del 35 al 58 → 23 s.
        let end = validate_range(35.0, 58.0, 120.0).unwrap();
        assert!((end - 35.0 - 23.0).abs() < 1e-9);
    }

    #[test]
    fn clips_are_new_files_next_to_the_original() {
        let dir = std::env::temp_dir().join(format!("strata-trim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("Partida 1.MP4");
        std::fs::write(&original, b"x").unwrap();
        assert_eq!(clip_path(&original), dir.join("Partida 1 (recorte).mp4"));
        std::fs::write(dir.join("Partida 1 (recorte).mp4"), b"x").unwrap();
        assert_eq!(clip_path(&original), dir.join("Partida 1 (recorte 2).mp4"));
        std::fs::write(dir.join("Partida 1 (recorte 2).mp4.trimming"), b"x").unwrap();
        assert_eq!(clip_path(&original), dir.join("Partida 1 (recorte 3).mp4"));
        assert!(original.exists(), "el original no se toca");
    }

    #[test]
    fn progress_lines_are_parsed() {
        assert_eq!(parse_progress_line("out_time_us=2500000"), Some(2.5));
        assert_eq!(parse_progress_line("out_time_ms=1000000"), Some(1.0));
        assert_eq!(parse_progress_line("out_time_us=N/A"), None);
        assert_eq!(parse_progress_line("progress=end"), None);
        assert_eq!(parse_progress_line("out_time_us=-5"), None);
    }
}
