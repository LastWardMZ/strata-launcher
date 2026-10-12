//! Biblioteca de vídeos: dónde se guardan, cómo se listan y cómo se protegen las rutas.
//!
//! Por defecto cada instancia guarda sus vídeos en `<instancia>/recordings`, igual que las capturas van a
//! `<instancia>/screenshots`. Con una carpeta de destino personalizada se crea una subcarpeta por instancia, así
//! los vídeos de instancias distintas nunca se mezclan.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::ffmpeg::MediaInfo;

pub const VIDEO_EXTENSIONS: [&str; 2] = ["mp4", "mkv"];
/// Grabación en curso (o interrumpida): no es un vídeo reproducible hasta que se finaliza.
pub const PARTIAL_EXTENSION: &str = "partial";

#[derive(Clone, Debug)]
pub struct Source {
    pub instance_id: String,
    pub instance_name: String,
    pub dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct VideoItem {
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub instance_id: String,
    pub instance_name: String,
    /// RFC 3339.
    pub created_at: String,
    pub size: u64,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_audio: Option<bool>,
}

/// Nombre válido como carpeta o archivo en Windows (sin caracteres prohibidos, sin nombres reservados).
pub fn sanitize_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).chars().take(60).collect::<String>();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    let stem = cleaned.split('.').next().unwrap_or("").to_uppercase();
    if cleaned.is_empty() || reserved.contains(&stem.as_str()) { format!("_{cleaned}") } else { cleaned }
}

/// Carpetas donde buscar vídeos. Con carpeta personalizada, cada instancia tiene su subcarpeta (con el
/// identificador si dos instancias se llaman igual).
pub fn sources(instances: &[(String, String, PathBuf)], custom_root: Option<&Path>) -> Vec<Source> {
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (_, name, _) in instances {
        *by_name.entry(sanitize_component(name)).or_default() += 1;
    }
    instances
        .iter()
        .map(|(id, name, path)| {
            let dir = match custom_root {
                Some(root) => {
                    let clean = sanitize_component(name);
                    let folder = if by_name.get(&clean).copied().unwrap_or(0) > 1 {
                        format!("{clean} ({})", sanitize_component(&id.chars().take(8).collect::<String>()))
                    } else {
                        clean
                    };
                    root.join(folder)
                }
                None => path.join("recordings"),
            };
            Source { instance_id: id.clone(), instance_name: name.clone(), dir }
        })
        .collect()
}

pub fn video_id(path: &Path) -> String {
    hex::encode(&Sha256::digest(path.to_string_lossy().to_lowercase().as_bytes())[..8])
}

pub fn is_video(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Ruta libre: `nombre.ext`, `nombre (2).ext`… Nunca devuelve un archivo que ya exista.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() && !dir.join(format!("{stem}.{ext}.{PARTIAL_EXTENSION}")).exists() {
        return first;
    }
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() && !dir.join(format!("{stem} ({n}).{ext}.{PARTIAL_EXTENSION}")).exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} ({}).{ext}", uuid::Uuid::new_v4().simple()))
}

pub fn timestamp_name(instance_name: &str) -> String {
    format!("{} {}", sanitize_component(instance_name), chrono::Local::now().format("%Y-%m-%d %H-%M-%S"))
}

/// Archivos de vídeo de una carpeta (sin recursión, sin ocultos ni temporales).
pub fn scan(dir: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let meta = entry.metadata().ok()?;
            let hidden = path.file_name()?.to_string_lossy().starts_with('.');
            (meta.is_file() && is_video(&path) && !hidden).then_some((path, meta))
        })
        .collect()
}

/// Temporal del audio que acompaña a un temporal de vídeo (`x.mp4.partial` → `x.mp4.partial-audio`). Su extensión es
/// distinta de `.partial` para que no se confunda con una grabación de vídeo.
pub fn audio_partial_path(video_partial: &Path) -> PathBuf {
    PathBuf::from(format!("{}-audio", video_partial.display()))
}

pub fn partials(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(PARTIAL_EXTENSION))
        .collect()
}

pub fn created_at(meta: &std::fs::Metadata) -> String {
    let time = meta.created().or_else(|_| meta.modified()).unwrap_or(UNIX_EPOCH);
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()
}

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

/// Una ruta es gestionable (reproducir, borrar, recortar) solo si es un vídeo dentro de una carpeta conocida
/// y sin escaparse mediante `..` ni enlaces.
pub fn is_managed(path: &Path, roots: &[PathBuf]) -> bool {
    if !is_video(path) || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    let Ok(real) = path.canonicalize() else { return false };
    roots.iter().any(|root| root.canonicalize().is_ok_and(|root| real.starts_with(&root)))
}

// ------------------------------------------------------------------ caché de metadatos

#[derive(Clone, Serialize, Deserialize, Debug)]
struct CachedInfo {
    size: u64,
    mtime: i64,
    info: MediaInfo,
}

/// Duración, resolución… de cada vídeo ya leídos con ffprobe. Se invalida si cambia el tamaño o la fecha.
#[derive(Default, Serialize, Deserialize)]
pub struct MetaCache {
    items: HashMap<String, CachedInfo>,
}

impl MetaCache {
    pub fn load(file: &Path) -> Self {
        std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) {
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(self) {
            let tmp = file.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, file);
            }
        }
    }

    pub fn get(&self, id: &str, meta: &std::fs::Metadata) -> Option<MediaInfo> {
        self.items.get(id).filter(|c| c.size == meta.len() && c.mtime == mtime_secs(meta)).map(|c| c.info.clone())
    }

    pub fn put(&mut self, id: &str, meta: &std::fs::Metadata, info: MediaInfo) {
        self.items.insert(id.to_string(), CachedInfo { size: meta.len(), mtime: mtime_secs(meta), info });
    }

    pub fn forget(&mut self, id: &str) {
        self.items.remove(id);
    }

    /// Elimina las entradas de vídeos que ya no existen.
    pub fn retain(&mut self, keep: &std::collections::HashSet<String>) {
        self.items.retain(|id, _| keep.contains(id));
    }
}

pub fn to_item(source: &Source, path: &Path, meta: &std::fs::Metadata, info: Option<&MediaInfo>) -> VideoItem {
    VideoItem {
        id: video_id(path),
        path: path.to_string_lossy().into_owned(),
        file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        instance_id: source.instance_id.clone(),
        instance_name: source.instance_name.clone(),
        created_at: created_at(meta),
        size: meta.len(),
        duration: info.and_then(|i| i.duration),
        width: info.and_then(|i| i.width),
        height: info.and_then(|i| i.height),
        has_audio: info.map(|i| i.has_audio),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("strata-lib-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_made_safe_for_windows() {
        assert_eq!(sanitize_component("Mi:Instancia*1?"), "Mi_Instancia_1_");
        assert_eq!(sanitize_component("  espacios.. "), "espacios");
        assert_eq!(sanitize_component("CON"), "_CON");
        assert_eq!(sanitize_component("aux.txt"), "_aux.txt");
        assert_eq!(sanitize_component(""), "_");
        assert_eq!(sanitize_component("../../x"), ".._.._x");
        assert!(sanitize_component(&"a".repeat(200)).len() <= 60);
    }

    #[test]
    fn default_sources_are_per_instance_and_custom_ones_never_mix() {
        let instances = vec![
            ("id-aaaaaaaa1".to_string(), "Survival".to_string(), PathBuf::from(r"C:\profiles\Survival")),
            ("id-bbbbbbbb2".to_string(), "Creativo".to_string(), PathBuf::from(r"C:\profiles\Creativo")),
            ("id-cccccccc3".to_string(), "Survival".to_string(), PathBuf::from(r"C:\profiles\Survival 2")),
        ];
        let by_default = sources(&instances, None);
        assert_eq!(by_default[0].dir, PathBuf::from(r"C:\profiles\Survival\recordings"));
        assert_ne!(by_default[0].dir, by_default[2].dir);
        let custom = sources(&instances, Some(Path::new(r"D:\Videos")));
        assert_eq!(custom[1].dir, PathBuf::from(r"D:\Videos\Creativo"));
        assert_ne!(custom[0].dir, custom[2].dir, "dos instancias con el mismo nombre no comparten carpeta");
        assert!(custom[0].dir.to_string_lossy().contains("id-aaaaa"));
    }

    #[test]
    fn unique_paths_never_collide_with_files_or_partials() {
        let dir = temp("unique");
        assert_eq!(unique_path(&dir, "clip", "mp4"), dir.join("clip.mp4"));
        std::fs::write(dir.join("clip.mp4"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "clip", "mp4"), dir.join("clip (2).mp4"));
        std::fs::write(dir.join("clip (2).mp4.partial"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "clip", "mp4"), dir.join("clip (3).mp4"));
    }

    #[test]
    fn scanning_lists_only_finished_videos() {
        let dir = temp("scan");
        for name in ["a.mp4", "b.MKV", "c.mp4.partial", "d.txt", ".oculto.mp4", "e.png"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::create_dir_all(dir.join("sub.mp4")).unwrap();
        let mut names: Vec<_> = scan(&dir).into_iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["a.mp4", "b.MKV"]);
        assert_eq!(partials(&dir).len(), 1);
        assert!(scan(&dir.join("no-existe")).is_empty());
    }

    #[test]
    fn only_videos_inside_known_folders_are_managed() {
        let root = temp("managed");
        let other = temp("outside");
        let video = root.join("v.mp4");
        std::fs::write(&video, b"x").unwrap();
        std::fs::write(other.join("v.mp4"), b"x").unwrap();
        std::fs::write(root.join("nota.txt"), b"x").unwrap();
        let roots = vec![root.clone()];
        assert!(is_managed(&video, &roots));
        assert!(!is_managed(&other.join("v.mp4"), &roots), "fuera de las carpetas conocidas");
        assert!(!is_managed(&root.join("nota.txt"), &roots), "no es un vídeo");
        assert!(!is_managed(&root.join("..").join(other.file_name().unwrap()).join("v.mp4"), &roots), "con ..");
        assert!(!is_managed(&root.join("no-existe.mp4"), &roots));
    }

    #[test]
    fn metadata_cache_invalidates_when_the_file_changes() {
        let dir = temp("cache");
        let file = dir.join("v.mp4");
        std::fs::write(&file, b"12345").unwrap();
        let meta = std::fs::metadata(&file).unwrap();
        let id = video_id(&file);
        let mut cache = MetaCache::default();
        assert!(cache.get(&id, &meta).is_none());
        let info = MediaInfo { duration: Some(3.5), width: Some(1280), height: Some(720), fps: Some(30.0), has_audio: true, video_codec: Some("h264".into()) };
        cache.put(&id, &meta, info.clone());
        assert_eq!(cache.get(&id, &meta), Some(info.clone()));
        let store = dir.join("meta.json");
        cache.save(&store);
        assert_eq!(MetaCache::load(&store).get(&id, &meta), Some(info));
        std::fs::write(&file, b"123456789").unwrap();
        assert!(MetaCache::load(&store).get(&id, &std::fs::metadata(&file).unwrap()).is_none(), "cambió el tamaño");
        assert!(MetaCache::load(&dir.join("roto.json")).items.is_empty());
    }

    #[test]
    fn ids_are_stable_and_case_insensitive_on_the_path() {
        assert_eq!(video_id(Path::new(r"C:\A\b.mp4")), video_id(Path::new(r"c:\a\B.MP4")));
        assert_ne!(video_id(Path::new(r"C:\A\b.mp4")), video_id(Path::new(r"C:\A\c.mp4")));
    }
}
