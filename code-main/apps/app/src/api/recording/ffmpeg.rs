//! Componente de FFmpeg: se descarga una sola vez (cuando el usuario activa la grabación), se verifica con una
//! huella SHA-256 fijada en el código y se guarda en la carpeta de datos de la app.
//!
//! Se usa la compilación «essentials» de gyan.dev (la que enlaza ffmpeg.org para Windows), publicada también en
//! GitHub con la misma huella. Es GPLv3 (incluye x264), compatible con la licencia de Strata (GPL-3.0). No se
//! redistribuye con el instalador: se descarga directamente de su origen.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

use super::args;

pub const VERSION: &str = "9.0.2";
pub const URL: &str = "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip";
pub const SHA256: &str = "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba";
pub const DOWNLOAD_SIZE: u64 = 114_768_076;
/// Espacio libre mínimo para descargar y descomprimir (zip + dos ejecutables).
pub const REQUIRED_FREE_BYTES: u64 = 450 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Paths {
    pub dir: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

pub fn paths(base: &Path) -> Paths {
    let dir = base.join("bin").join("ffmpeg").join(VERSION);
    Paths { ffmpeg: dir.join("ffmpeg.exe"), ffprobe: dir.join("ffprobe.exe"), dir }
}

pub fn is_installed(base: &Path) -> bool {
    let p = paths(base);
    let big = |path: &Path| std::fs::metadata(path).is_ok_and(|m| m.len() > 10 * 1024 * 1024);
    big(&p.ffmpeg) && big(&p.ffprobe)
}

#[derive(Clone, Serialize, Debug)]
pub struct Progress {
    /// `downloading`, `verifying`, `extracting`, `checking` o `done`.
    pub phase: &'static str,
    pub downloaded: u64,
    pub total: u64,
}

/// Comando de FFmpeg sin ventana de consola.
pub fn command(program: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd.kill_on_drop(true);
    cmd
}

/// Descarga, verifica y extrae FFmpeg. `progress` se llama con cada avance.
pub async fn install(base: &Path, progress: impl Fn(Progress) + Send + Sync + 'static) -> Result<(), String> {
    if is_installed(base) {
        return Ok(());
    }
    let target = paths(base);
    let work = base.join("bin").join(format!("descarga-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&work).await.map_err(|e| format!("No se pudo crear la carpeta temporal: {e}"))?;
    let result = install_inner(&work, &target, &progress).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    if result.is_err() {
        // Una instalación a medias no debe dejar ejecutables sueltos.
        let _ = tokio::fs::remove_file(&target.ffmpeg).await;
        let _ = tokio::fs::remove_file(&target.ffprobe).await;
    }
    result
}

async fn install_inner(work: &Path, target: &Paths, progress: &(impl Fn(Progress) + Send + Sync)) -> Result<(), String> {
    if let Some(free) = super::free_space(work) {
        if free < REQUIRED_FREE_BYTES {
            return Err(format!(
                "No hay espacio suficiente: se necesitan unos {} MB libres y hay {} MB.",
                REQUIRED_FREE_BYTES / 1_048_576,
                free / 1_048_576
            ));
        }
    }
    let zip_path = work.join("ffmpeg.zip");
    let mut file = tokio::fs::File::create(&zip_path).await.map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .user_agent("Strata-Launcher (https://github.com/LastWardMZ/strata-launcher)")
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("No se pudo descargar FFmpeg: {e}"))?;
    let total = response.content_length().unwrap_or(DOWNLOAD_SIZE);
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(60), response.chunk())
            .await
            .map_err(|_| "La descarga de FFmpeg se ha quedado parada".to_string())?
            .map_err(|e| format!("Error durante la descarga: {e}"))?;
        let Some(chunk) = chunk else { break };
        downloaded += chunk.len() as u64;
        if downloaded > DOWNLOAD_SIZE + 1024 * 1024 {
            return Err("El archivo descargado es más grande de lo esperado".into());
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| format!("No se pudo guardar la descarga: {e}"))?;
        progress(Progress { phase: "downloading", downloaded, total });
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);

    progress(Progress { phase: "verifying", downloaded, total });
    let digest = hex::encode(hasher.finalize());
    if digest != SHA256 {
        return Err(format!(
            "La descarga no coincide con la huella esperada ({}…): se ha descartado por seguridad.",
            &digest[..12]
        ));
    }

    progress(Progress { phase: "extracting", downloaded, total });
    let (zip, dir) = (zip_path.clone(), target.dir.clone());
    tokio::task::spawn_blocking(move || extract(&zip, &dir)).await.map_err(|e| e.to_string())??;

    progress(Progress { phase: "checking", downloaded, total });
    check_binary(&target.ffmpeg).await?;
    progress(Progress { phase: "done", downloaded, total });
    Ok(())
}

/// Extrae solo `ffmpeg.exe`, `ffprobe.exe` y la licencia, siempre con nombres fijos (nunca se usa la ruta del zip).
fn extract(zip_path: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("El archivo de FFmpeg no es válido: {e}"))?;
    let wanted = [("/bin/ffmpeg.exe", "ffmpeg.exe"), ("/bin/ffprobe.exe", "ffprobe.exe"), ("/LICENSE", "LICENSE.txt")];
    let mut found = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let Some((_, out_name)) = wanted.iter().find(|(suffix, _)| name.ends_with(suffix) && name.matches('/').count() <= 2) else {
            continue;
        };
        let tmp = dir.join(format!("{out_name}.tmp"));
        let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("No se pudo extraer {out_name}: {e}"))?;
        drop(out);
        std::fs::rename(&tmp, dir.join(out_name)).map_err(|e| e.to_string())?;
        found += 1;
    }
    if found < 2 {
        return Err("El archivo de FFmpeg no contiene los ejecutables esperados".into());
    }
    Ok(())
}

/// Comprueba que el ejecutable arranca, es la versión esperada y trae la captura de ventana por GPU.
pub async fn check_binary(ffmpeg: &Path) -> Result<(), String> {
    let run = |args: &'static [&'static str]| {
        let mut cmd = command(ffmpeg);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        async move {
            let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
                .await
                .map_err(|_| "FFmpeg no responde".to_string())?
                .map_err(|e| format!("No se pudo ejecutar FFmpeg: {e}"))?;
            Ok::<String, String>(String::from_utf8_lossy(&out.stdout).into_owned())
        }
    };
    let version = run(&["-hide_banner", "-version"]).await?;
    if !version.contains(VERSION) {
        return Err("La versión de FFmpeg no es la esperada".into());
    }
    let filters = run(&["-hide_banner", "-filters"]).await?;
    if !filters.contains("gfxcapture") {
        return Err("Este FFmpeg no incluye la captura de ventanas (gfxcapture)".into());
    }
    Ok(())
}

#[derive(Clone, Serialize, Debug)]
pub struct EncoderStatus {
    pub name: &'static str,
    pub label: &'static str,
    pub hardware: bool,
    pub available: bool,
}

/// Prueba cada codificador con un vídeo sintético: que figure en la lista no significa que la tarjeta lo soporte
/// (por ejemplo NVENC exige un controlador reciente).
pub async fn probe_encoders(ffmpeg: &Path) -> Vec<EncoderStatus> {
    let mut found = Vec::new();
    for name in args::ENCODER_CANDIDATES {
        let mut cmd = command(ffmpeg);
        cmd.args(args::probe_encoder_args(name)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let ok = matches!(tokio::time::timeout(Duration::from_secs(20), cmd.status()).await, Ok(Ok(status)) if status.success());
        found.push(EncoderStatus { name, label: args::encoder_label(name), hardware: args::is_hardware(name), available: ok });
    }
    found
}

/// Elige el codificador según el modo de los ajustes. `None` si no hay ninguno válido.
pub fn choose_encoder(mode: &str, available: &[EncoderStatus]) -> Option<&'static str> {
    available
        .iter()
        .filter(|e| e.available)
        .find(|e| match mode {
            "hardware" => e.hardware,
            "software" => !e.hardware,
            _ => true,
        })
        .map(|e| e.name)
}

#[derive(Clone, Default, Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_audio: bool,
    pub video_codec: Option<String>,
}

pub fn parse_probe(json: &str) -> Result<MediaInfo, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut info = MediaInfo::default();
    info.duration = value["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()).filter(|d| d.is_finite() && *d >= 0.0);
    for stream in value["streams"].as_array().into_iter().flatten() {
        match stream["codec_type"].as_str() {
            Some("video") if info.width.is_none() => {
                info.width = stream["width"].as_u64().map(|v| v as u32);
                info.height = stream["height"].as_u64().map(|v| v as u32);
                info.video_codec = stream["codec_name"].as_str().map(str::to_string);
                info.fps = stream["avg_frame_rate"].as_str().and_then(parse_rate);
                if info.duration.is_none() {
                    info.duration = stream["duration"].as_str().and_then(|d| d.parse().ok());
                }
            }
            Some("audio") => info.has_audio = true,
            _ => {}
        }
    }
    if info.width.is_none() {
        return Err("El archivo no contiene vídeo".into());
    }
    Ok(info)
}

fn parse_rate(rate: &str) -> Option<f64> {
    let (n, d) = rate.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

pub async fn probe_media(ffprobe: &Path, file: &Path) -> Result<MediaInfo, String> {
    let mut cmd = command(ffprobe);
    cmd.args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,codec_name,width,height,avg_frame_rate,duration", "-of", "json"])
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| "ffprobe no responde".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("No se pudo leer el vídeo (¿está dañado?)".into());
    }
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_download_is_well_formed() {
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(URL.starts_with("https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/"));
        assert!(URL.contains(VERSION));
        let p = paths(Path::new(r"C:\datos"));
        assert!(p.ffmpeg.ends_with(r"bin\ffmpeg\9.0.2\ffmpeg.exe") || p.ffmpeg.to_string_lossy().contains("9.0.2"));
    }

    #[test]
    fn encoder_choice_respects_the_mode_and_availability() {
        let status = |name: &'static str, available: bool| EncoderStatus { name, label: "", hardware: args::is_hardware(name), available };
        let all = [status("h264_nvenc", false), status("h264_amf", false), status("h264_qsv", false), status("h264_mf", true), status("libx264", true)];
        assert_eq!(choose_encoder("auto", &all), Some("h264_mf"));
        assert_eq!(choose_encoder("hardware", &all), Some("h264_mf"));
        assert_eq!(choose_encoder("software", &all), Some("libx264"));
        let only_software = [status("h264_nvenc", false), status("libx264", true)];
        assert_eq!(choose_encoder("hardware", &only_software), None, "pide hardware y no hay");
        assert_eq!(choose_encoder("auto", &only_software), Some("libx264"));
        assert_eq!(choose_encoder("auto", &[]), None);
    }

    #[test]
    fn ffprobe_output_is_parsed() {
        let json = r#"{"streams":[{"codec_name":"h264","codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"60/1"},{"codec_name":"aac","codec_type":"audio"}],"format":{"duration":"12.345000"}}"#;
        let info = parse_probe(json).unwrap();
        assert_eq!((info.width, info.height), (Some(1920), Some(1080)));
        assert_eq!(info.duration, Some(12.345));
        assert_eq!(info.fps, Some(60.0));
        assert!(info.has_audio);
        assert_eq!(info.video_codec.as_deref(), Some("h264"));
        assert!(parse_probe(r#"{"streams":[{"codec_type":"audio"}],"format":{}}"#).is_err(), "sin vídeo");
        assert!(parse_probe("no json").is_err());
        assert_eq!(parse_probe(r#"{"streams":[{"codec_type":"video","width":2,"height":2,"avg_frame_rate":"0/0"}],"format":{}}"#).unwrap().fps, None);
    }

    /// Prueba manual (ignorada): descarga de verdad FFmpeg (~110 MB), comprueba la huella, lo extrae y lo ejecuta.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "manual: descarga unos 110 MB"]
    async fn installs_the_pinned_ffmpeg_for_real() {
        let base = std::env::temp_dir().join(format!("strata-ff-real-{}", uuid::Uuid::new_v4()));
        let phases = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let seen = phases.clone();
        let started = std::time::Instant::now();
        install(&base, move |p| {
            let mut seen = seen.lock().unwrap();
            if seen.last() != Some(&p.phase) {
                seen.push(p.phase);
            }
        })
        .await
        .expect("instalar FFmpeg");
        println!("INSTALADO en {:.1} s; fases: {:?}", started.elapsed().as_secs_f64(), phases.lock().unwrap());
        assert!(is_installed(&base));
        assert_eq!(phases.lock().unwrap().as_slice(), ["downloading", "verifying", "extracting", "checking", "done"]);
        let found = probe_encoders(&paths(&base).ffmpeg).await;
        println!("CODIFICADORES: {:?}", found.iter().map(|e| (e.name, e.available)).collect::<Vec<_>>());
        assert!(found.iter().any(|e| e.name == "libx264" && e.available), "x264 siempre funciona");
        // Volver a instalar no descarga nada.
        let again = std::time::Instant::now();
        install(&base, |_| {}).await.unwrap();
        assert!(again.elapsed() < std::time::Duration::from_secs(2));
        assert!(paths(&base).dir.join("LICENSE.txt").exists(), "se conserva la licencia");
    }

    #[test]
    fn extraction_only_takes_known_files_with_fixed_names() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("strata-ff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("test.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe", "EXE1"),
                ("ffmpeg-9.0.2-essentials_build/bin/ffprobe.exe", "EXE2"),
                ("ffmpeg-9.0.2-essentials_build/bin/ffplay.exe", "NO"),
                ("ffmpeg-9.0.2-essentials_build/LICENSE", "GPL"),
                ("../../evil.exe", "MAL"),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        let out = dir.join("out");
        extract(&zip_path, &out).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("ffmpeg.exe")).unwrap(), "EXE1");
        assert_eq!(std::fs::read_to_string(out.join("ffprobe.exe")).unwrap(), "EXE2");
        assert!(out.join("LICENSE.txt").exists());
        assert!(!out.join("ffplay.exe").exists() && !dir.join("evil.exe").exists() && !out.join("evil.exe").exists());
        // Un zip sin los ejecutables se rechaza.
        let empty = dir.join("empty.zip");
        zip::ZipWriter::new(std::fs::File::create(&empty).unwrap()).finish().unwrap();
        assert!(extract(&empty, &dir.join("out2")).is_err());
    }
}
