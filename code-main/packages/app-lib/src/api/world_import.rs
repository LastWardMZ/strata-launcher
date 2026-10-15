//! Instalar un mundo (mapa) desde un ZIP local en la carpeta `saves` de una instancia.
//!
//! Este módulo no sabe de dónde viene el archivo (CurseForge, un ZIP que el usuario ya tenía u otra fuente futura): solo
//! lo inspecciona y lo instala. **El ZIP se trata como hostil**:
//! - Zip slip: se rechazan rutas absolutas, `..`, letras de unidad, rutas UNC, nombres reservados de Windows y flujos
//!   alternativos (`:`). Cada entrada se escribe solo dentro de una carpeta de trabajo nueva.
//! - No se siguen ni se crean enlaces simbólicos: un ZIP que los lleve se rechaza.
//! - Límites contra bombas: tamaño total descomprimido, número de archivos y ratio de compresión, comprobados sobre lo que
//!   declara el ZIP y otra vez sobre los bytes realmente escritos. Se comprueba el espacio libre antes de escribir.
//! - Se descomprime en una carpeta temporal dentro de la instancia, se valida y solo entonces se mueve a `saves`. Si algo
//!   falla se limpia todo y no queda ningún mundo a medias. Nunca se sobrescribe un mundo existente: se añade " (2)".
//! - Nada se ejecuta. Los archivos ejecutables (`.jar`, `.exe`, `.bat`…) no se copian y se devuelven en un aviso.
//! - `level.dat` se lee con límites de descompresión y con un lector de NBT que no reserva memoria según lo que diga el
//!   archivo.

use crate::State;
use crate::event::LoadingBarType;
use crate::event::emit::{emit_loading, init_loading};
use crate::state::attached_world_data;
use crate::worlds::WorldType;
use serde::Serialize;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MIB: u64 = 1024 * 1024;

/// Límites de seguridad al leer un ZIP.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Tamaño total descomprimido.
    pub max_total_bytes: u64,
    /// Número de archivos.
    pub max_files: usize,
    /// Ratio descomprimido/comprimido (solo se aplica a partir de 1 MiB: un ZIP minúsculo puede comprimir muy bien).
    pub max_ratio: u64,
    /// Tamaño máximo de `level.dat` descomprimido.
    pub max_level_dat: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_total_bytes: 4 * 1024 * MIB, max_files: 100_000, max_ratio: 200, max_level_dat: 4 * MIB }
    }
}

/// Extensiones que no se copian nunca, por si alguien intenta colar un programa en un mapa.
const BLOCKED_EXTENSIONS: &[&str] = &["jar", "exe", "bat", "cmd", "ps1", "vbs", "dll", "scr", "lnk", "com", "msi"];
/// Cuántos nombres bloqueados se devuelven en el aviso.
const MAX_BLOCKED_REPORTED: usize = 50;

#[derive(Debug, PartialEq, Eq)]
pub enum ImportError {
    /// El archivo no es un ZIP legible.
    NotAZip,
    /// Una entrada intenta salir de la carpeta del mundo (zip slip), es un enlace o tiene un nombre peligroso.
    Unsafe(String),
    TooBig,
    TooManyFiles,
    /// Comprime sospechosamente bien (posible bomba de descompresión) o miente sobre su tamaño.
    Suspicious,
    /// No hay ningún `level.dat` válido: no es un mundo de Minecraft.
    NoWorld,
    /// Hay varios mundos y no se ha dicho cuál instalar.
    NeedsSelection,
    /// Se pidió un mundo que no está en el ZIP.
    UnknownWorld(String),
    NoSpace { needed: u64, free: u64 },
    Io(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAZip => write!(f, "El archivo no es un ZIP válido."),
            Self::Unsafe(name) => write!(f, "El ZIP contiene una ruta peligrosa («{name}») y no se ha instalado."),
            Self::TooBig => write!(f, "El mapa descomprimido es demasiado grande."),
            Self::TooManyFiles => write!(f, "El ZIP contiene demasiados archivos."),
            Self::Suspicious => write!(f, "El ZIP parece una bomba de compresión y no se ha instalado."),
            Self::NoWorld => write!(f, "El ZIP no contiene ningún mundo de Minecraft (no se encuentra level.dat)."),
            Self::NeedsSelection => write!(f, "El ZIP trae varios mundos: elige cuáles instalar."),
            Self::UnknownWorld(name) => write!(f, "El ZIP no contiene el mundo «{name}»."),
            Self::NoSpace { needed, free } => write!(
                f,
                "No hay espacio suficiente en el disco: hacen falta {} MB y hay {} MB libres.",
                needed.div_ceil(MIB),
                free / MIB
            ),
            Self::Io(message) => write!(f, "No se pudo instalar el mapa: {message}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<ImportError> for crate::Error {
    fn from(error: ImportError) -> Self {
        crate::ErrorKind::OtherError(error.to_string()).into()
    }
}

fn io_error(error: impl std::fmt::Display) -> ImportError {
    ImportError::Io(error.to_string())
}

/// Un mundo encontrado dentro del ZIP.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorldInZip {
    /// Carpeta del mundo dentro del ZIP (vacía si está en la raíz). Es el identificador para elegirlo.
    pub path: String,
    /// Nombre de carpeta con el que se instalará (sin " (2)" si ya existe).
    pub folder: String,
    pub level_name: Option<String>,
    /// Versión de Minecraft con la que se guardó, si `level.dat` la dice.
    pub game_version: Option<String>,
    pub data_version: Option<i32>,
    /// Tamaño descomprimido y número de archivos que se instalarían (sin los bloqueados).
    pub size: u64,
    pub files: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ZipInspection {
    pub worlds: Vec<WorldInZip>,
    /// Ejecutables u otros archivos que no se copiarán (como mucho `MAX_BLOCKED_REPORTED`).
    pub blocked_files: Vec<String>,
    pub total_size: u64,
    pub entries: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InstalledWorld {
    /// Carpeta creada en `saves`.
    pub folder: String,
    pub level_name: Option<String>,
    pub game_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InstallReport {
    pub installed: Vec<InstalledWorld>,
    pub blocked_files: Vec<String>,
}

/// De dónde viene el mundo, para recordarlo en los datos locales de la instancia.
#[derive(Debug, Clone)]
pub struct WorldOrigin {
    pub project_id: String,
    pub content_kind: String,
}

// ───────────────────────── Rutas seguras ─────────────────────────

fn is_reserved_windows_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0')
}

/// Descompone el nombre de una entrada en componentes seguros, o la rechaza (zip slip y nombres peligrosos).
fn safe_components(raw: &str) -> Result<Vec<String>, ImportError> {
    let unsafe_name = || ImportError::Unsafe(raw.chars().take(80).collect());
    if raw.is_empty() || raw.contains('\0') {
        return Err(unsafe_name());
    }
    let name = raw.replace('\\', "/");
    // Absolutas y UNC (`/x`, `//servidor/x`) y letras de unidad (`C:x`).
    if name.starts_with('/') {
        return Err(unsafe_name());
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(unsafe_name());
    }
    let mut parts = Vec::new();
    for component in name.split('/') {
        match component {
            "" | "." => continue,
            ".." => return Err(unsafe_name()),
            _ => {}
        }
        if component.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c))
            || component.ends_with('.')
            || component.ends_with(' ')
            || is_reserved_windows_name(component)
        {
            return Err(unsafe_name());
        }
        parts.push(component.to_string());
    }
    Ok(parts)
}

/// Nombre de carpeta válido en Windows, Linux y macOS a partir de un texto cualquiera.
pub fn sanitize_folder_name(raw: &str) -> String {
    let cleaned: String = raw.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim_end_matches(['.', ' ']);
    let limited: String = trimmed.chars().take(64).collect();
    let limited = limited.trim_end_matches(['.', ' ']).to_string();
    if limited.is_empty() || is_reserved_windows_name(&limited) { "Mapa".to_string() } else { limited }
}

fn is_blocked(parts: &[String]) -> bool {
    parts
        .last()
        .and_then(|name| Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| BLOCKED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

fn is_junk(parts: &[String]) -> bool {
    parts.iter().any(|part| part == "__MACOSX" || part.starts_with("._"))
}

// ───────────────────────── Lector de level.dat ─────────────────────────

struct LevelInfo {
    level_name: Option<String>,
    game_version: Option<String>,
    data_version: Option<i32>,
}

/// Lector de NBT que solo avanza por bytes que existen: nunca reserva memoria según lo que diga el archivo.
struct Nbt<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Nbt<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(count)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.take(4).map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn string(&mut self) -> Option<String> {
        let length = usize::from(self.u16()?);
        self.take(length).map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }
    fn skip_array(&mut self, element: usize) -> Option<()> {
        let length = usize::try_from(self.i32()?).ok()?;
        self.take(length.checked_mul(element)?).map(|_| ())
    }
    fn skip(&mut self, tag: u8, depth: u32) -> Option<()> {
        if depth > 32 {
            return None;
        }
        match tag {
            1 => self.take(1).map(|_| ()),
            2 => self.take(2).map(|_| ()),
            3 | 5 => self.take(4).map(|_| ()),
            4 | 6 => self.take(8).map(|_| ()),
            7 => self.skip_array(1),
            8 => {
                let length = usize::from(self.u16()?);
                self.take(length).map(|_| ())
            }
            9 => {
                let element = self.u8()?;
                let length = usize::try_from(self.i32()?).ok()?;
                if element == 0 {
                    return (length == 0).then_some(());
                }
                // Cada elemento ocupa al menos un byte, así que esto acaba en cuanto se acaban los datos.
                for _ in 0..length {
                    self.skip(element, depth + 1)?;
                }
                Some(())
            }
            10 => loop {
                let inner = self.u8()?;
                if inner == 0 {
                    return Some(());
                }
                self.string()?;
                self.skip(inner, depth + 1)?;
            },
            11 => self.skip_array(4),
            12 => self.skip_array(8),
            _ => None,
        }
    }
}

/// `Data.LevelName`, `Data.DataVersion` y `Data.Version.Name` de un `level.dat` ya descomprimido.
fn read_level_info(data: &[u8]) -> Option<LevelInfo> {
    let mut nbt = Nbt { data, pos: 0 };
    if nbt.u8()? != 10 {
        return None;
    }
    nbt.string()?;
    let mut found = None;
    loop {
        let tag = nbt.u8()?;
        if tag == 0 {
            return found;
        }
        let name = nbt.string()?;
        if tag == 10 && name == "Data" {
            let mut info = LevelInfo { level_name: None, game_version: None, data_version: None };
            loop {
                let inner = nbt.u8()?;
                if inner == 0 {
                    break;
                }
                let key = nbt.string()?;
                match (inner, key.as_str()) {
                    (8, "LevelName") => info.level_name = Some(nbt.string()?),
                    (3, "DataVersion") => info.data_version = Some(nbt.i32()?),
                    (10, "Version") => loop {
                        let version_tag = nbt.u8()?;
                        if version_tag == 0 {
                            break;
                        }
                        let version_key = nbt.string()?;
                        if version_tag == 8 && version_key == "Name" {
                            info.game_version = Some(nbt.string()?);
                        } else {
                            nbt.skip(version_tag, 0)?;
                        }
                    },
                    _ => nbt.skip(inner, 0)?,
                }
            }
            found = Some(info);
        } else {
            nbt.skip(tag, 0)?;
        }
    }
}

/// Descomprime un `level.dat` (gzip) con límite de tamaño.
fn read_level_dat(source: impl Read, limits: &Limits) -> Result<Option<LevelInfo>, ImportError> {
    let mut decoder = flate2::read::GzDecoder::new(source.take(limits.max_level_dat)).take(limits.max_level_dat + 1);
    let mut raw = Vec::new();
    if decoder.read_to_end(&mut raw).is_err() {
        return Ok(None); // no es un gzip: no vale como mundo
    }
    if raw.len() as u64 > limits.max_level_dat {
        return Err(ImportError::Suspicious);
    }
    Ok(read_level_info(&raw))
}

// ───────────────────────── Inspección ─────────────────────────

struct Entry {
    index: usize,
    parts: Vec<String>,
    size: u64,
}

struct Scan {
    entries: Vec<Entry>,
    blocked: Vec<String>,
    total: u64,
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<io::BufReader<fs::File>>, ImportError> {
    let file = fs::File::open(path).map_err(io_error)?;
    zip::ZipArchive::new(io::BufReader::new(file)).map_err(|_| ImportError::NotAZip)
}

/// Recorre el directorio central (sin descomprimir nada) y aplica todas las comprobaciones de seguridad.
fn scan(archive: &mut zip::ZipArchive<io::BufReader<fs::File>>, limits: &Limits) -> Result<Scan, ImportError> {
    if archive.len() > limits.max_files {
        return Err(ImportError::TooManyFiles);
    }
    let (mut entries, mut blocked) = (Vec::new(), Vec::new());
    let (mut total, mut compressed) = (0u64, 0u64);
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(|_| ImportError::NotAZip)?;
        let name = entry.name().to_string();
        let parts = safe_components(&name)?;
        if entry.unix_mode().is_some_and(|mode| mode & 0o170_000 == 0o120_000) {
            return Err(ImportError::Unsafe(name.chars().take(80).collect()));
        }
        if entry.is_dir() || parts.is_empty() {
            continue;
        }
        total = total.checked_add(entry.size()).ok_or(ImportError::TooBig)?;
        compressed = compressed.saturating_add(entry.compressed_size());
        if total > limits.max_total_bytes {
            return Err(ImportError::TooBig);
        }
        if is_junk(&parts) {
            continue;
        }
        if is_blocked(&parts) {
            if blocked.len() < MAX_BLOCKED_REPORTED {
                blocked.push(parts.join("/"));
            }
            continue;
        }
        entries.push(Entry { index, parts, size: entry.size() });
    }
    if total > MIB && total / compressed.max(1) > limits.max_ratio {
        return Err(ImportError::Suspicious);
    }
    Ok(Scan { entries, blocked, total })
}

/// Carpetas que contienen un `level.dat`, quitando las que están dentro de otro mundo.
fn world_dirs(entries: &[Entry]) -> Vec<(Vec<String>, usize)> {
    let mut found: Vec<(Vec<String>, usize)> = entries
        .iter()
        .filter(|entry| entry.parts.last().is_some_and(|name| name.eq_ignore_ascii_case("level.dat")))
        .map(|entry| (entry.parts[..entry.parts.len() - 1].to_vec(), entry.index))
        .collect();
    found.sort();
    let dirs: Vec<Vec<String>> = found.iter().map(|(dir, _)| dir.clone()).collect();
    found.retain(|(dir, _)| !dirs.iter().any(|other| other.len() < dir.len() && dir.starts_with(other)));
    found.dedup_by(|a, b| a.0 == b.0);
    found
}

fn folder_for(dir: &[String], level_name: Option<&str>) -> String {
    sanitize_folder_name(dir.last().map(String::as_str).or(level_name).unwrap_or("Mapa"))
}

pub fn inspect(zip_path: &Path) -> Result<ZipInspection, ImportError> {
    inspect_with(zip_path, &Limits::default())
}

pub fn inspect_with(zip_path: &Path, limits: &Limits) -> Result<ZipInspection, ImportError> {
    let mut archive = open_zip(zip_path)?;
    let scan = scan(&mut archive, limits)?;
    let mut worlds = Vec::new();
    for (dir, level_index) in world_dirs(&scan.entries) {
        let entry = archive.by_index(level_index).map_err(|_| ImportError::NotAZip)?;
        let Some(info) = read_level_dat(entry, limits)? else { continue };
        let inside: Vec<&Entry> = scan.entries.iter().filter(|e| e.parts.starts_with(&dir)).collect();
        worlds.push(WorldInZip {
            path: dir.join("/"),
            folder: folder_for(&dir, info.level_name.as_deref()),
            level_name: info.level_name,
            game_version: info.game_version,
            data_version: info.data_version,
            size: inside.iter().map(|e| e.size).sum(),
            files: inside.len(),
        });
    }
    if worlds.is_empty() {
        return Err(ImportError::NoWorld);
    }
    Ok(ZipInspection { worlds, blocked_files: scan.blocked, total_size: scan.total, entries: scan.entries.len() })
}

// ───────────────────────── Instalación ─────────────────────────

/// Primer nombre libre en `saves`: `Mapa`, `Mapa (2)`, `Mapa (3)`…
fn unique_folder(saves: &Path, wanted: &str) -> String {
    let taken = |name: &str| fs::symlink_metadata(saves.join(name)).is_ok();
    if !taken(wanted) {
        return wanted.to_string();
    }
    (2u32..)
        .map(|n| format!("{wanted} ({n})"))
        .find(|candidate| !taken(candidate))
        .expect("siempre hay un nombre libre")
}

/// Copia `source` (limitado a `declared + 1` bytes) a un archivo nuevo; falla si pesa más de lo que declaraba.
fn write_entry(mut source: impl Read, destination: &Path, declared: u64, written_total: &mut u64, limits: &Limits) -> Result<(), ImportError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(destination).map_err(io_error)?;
    let mut limited = (&mut source).take(declared.saturating_add(1));
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    loop {
        let read = limited.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        written += read as u64;
        *written_total += read as u64;
        if written > declared || *written_total > limits.max_total_bytes {
            return Err(ImportError::Suspicious);
        }
        file.write_all(&buffer[..read]).map_err(io_error)?;
    }
    Ok(())
}

/// Instala los mundos elegidos (`selected`: carpetas dentro del ZIP, como en `WorldInZip::path`; con un solo mundo en el
/// ZIP vale una lista vacía). `instance_dir` es la carpeta de la instancia; `progress` recibe una fracción de 0 a 1.
pub fn install(
    zip_path: &Path,
    instance_dir: &Path,
    selected: &[String],
    limits: &Limits,
    progress: &mut dyn FnMut(f64),
) -> Result<InstallReport, ImportError> {
    let mut archive = open_zip(zip_path)?;
    let scan = scan(&mut archive, limits)?;
    let candidates = world_dirs(&scan.entries);

    // Mundos válidos del ZIP y cuáles se van a instalar.
    let mut worlds: Vec<(Vec<String>, LevelInfo)> = Vec::new();
    for (dir, level_index) in candidates {
        let entry = archive.by_index(level_index).map_err(|_| ImportError::NotAZip)?;
        if let Some(info) = read_level_dat(entry, limits)? {
            worlds.push((dir, info));
        }
    }
    if worlds.is_empty() {
        return Err(ImportError::NoWorld);
    }
    let chosen: Vec<&(Vec<String>, LevelInfo)> = if selected.is_empty() {
        if worlds.len() > 1 {
            return Err(ImportError::NeedsSelection);
        }
        worlds.iter().collect()
    } else {
        let mut chosen = Vec::new();
        for wanted in selected {
            let found = worlds.iter().find(|(dir, _)| dir.join("/") == *wanted);
            chosen.push(found.ok_or_else(|| ImportError::UnknownWorld(wanted.clone()))?);
        }
        chosen
    };

    // Carpeta `saves` real de esta instancia: no puede ser un enlace ni salir de ella.
    let instance_real = fs::canonicalize(instance_dir).map_err(io_error)?;
    let saves = instance_dir.join("saves");
    if let Ok(metadata) = fs::symlink_metadata(&saves) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ImportError::Unsafe("saves".into()));
        }
    } else {
        fs::create_dir_all(&saves).map_err(io_error)?;
    }
    if !fs::canonicalize(&saves).map_err(io_error)?.starts_with(&instance_real) {
        return Err(ImportError::Unsafe("saves".into()));
    }

    // Espacio en disco para lo que se va a descomprimir.
    let needed: u64 = chosen
        .iter()
        .map(|(dir, _)| scan.entries.iter().filter(|e| e.parts.starts_with(dir)).map(|e| e.size).sum::<u64>())
        .sum();
    let free = fs4::available_space(&instance_real).map_err(io_error)?;
    if free < needed.saturating_add(64 * MIB) {
        return Err(ImportError::NoSpace { needed: needed.saturating_add(64 * MIB), free });
    }

    // Se descomprime fuera de `saves`, y solo cuando todo es válido se mueve al sitio.
    let staging = tempfile::Builder::new().prefix(".strata-mapa-").tempdir_in(&instance_real).map_err(io_error)?;
    let mut written_total = 0u64;
    let mut staged: Vec<(PathBuf, String, &LevelInfo)> = Vec::new();
    for (position, (dir, info)) in chosen.iter().map(|pair| (&pair.0, &pair.1)).enumerate() {
        let root = staging.path().join(position.to_string());
        for entry in scan.entries.iter().filter(|e| e.parts.starts_with(dir)) {
            let destination = entry.parts[dir.len()..].iter().fold(root.clone(), |path, part| path.join(part));
            debug_assert!(destination.starts_with(&root));
            let source = archive.by_index(entry.index).map_err(|_| ImportError::NotAZip)?;
            write_entry(source, &destination, entry.size, &mut written_total, limits)?;
            progress(written_total as f64 / needed.max(1) as f64 * 0.95);
        }
        if !root.join("level.dat").is_file() {
            return Err(ImportError::NoWorld);
        }
        staged.push((root, folder_for(dir, info.level_name.as_deref()), info));
    }

    let mut installed = Vec::new();
    for (root, wanted, info) in staged {
        let folder = unique_folder(&saves, &wanted);
        fs::rename(&root, saves.join(&folder)).map_err(io_error)?;
        installed.push(InstalledWorld { folder, level_name: info.level_name.clone(), game_version: info.game_version.clone() });
    }
    progress(1.0);
    Ok(InstallReport { installed, blocked_files: scan.blocked })
}

// ───────────────────────── Con instancias reales ─────────────────────────

/// Inspecciona un ZIP (en un hilo aparte: puede pesar cientos de MB).
pub async fn inspect_zip(zip_path: PathBuf) -> crate::Result<ZipInspection> {
    tokio::task::spawn_blocking(move || inspect(&zip_path))
        .await
        .map_err(|e| crate::ErrorKind::OtherError(e.to_string()))?
        .map_err(Into::into)
}

/// Instala los mundos elegidos de un ZIP en `saves` de una instancia que existe, con progreso en el panel de tareas, y
/// recuerda su origen en los datos locales de cada mundo.
pub async fn install_into_instance(
    instance_id: &str,
    zip_path: PathBuf,
    selected: Vec<String>,
    origin: Option<WorldOrigin>,
) -> crate::Result<InstallReport> {
    // Falla si la instancia no existe: el destino nunca viene de una ruta que dé la interfaz.
    let instance_dir = crate::instance::get_full_path(instance_id).await?;
    let instance_name = crate::instance::get(instance_id).await?.map(|metadata| metadata.instance.name).unwrap_or_else(|| instance_id.to_string());
    let bar = init_loading(LoadingBarType::ZipExtract { instance_id: instance_id.to_string(), instance_name }, 100.0, "Instalando mapa…").await?;

    let (result, bar, last) = tokio::task::spawn_blocking(move || {
        let mut last = 0.0;
        let result = install(&zip_path, &instance_dir, &selected, &Limits::default(), &mut |fraction| {
            let step = (fraction * 100.0 - last).max(0.0);
            if step >= 1.0 {
                last += step;
                let _ = emit_loading(&bar, step, None);
            }
        });
        (result, bar, last)
    })
    .await
    .map_err(|e| crate::ErrorKind::OtherError(e.to_string()))?;
    // La barra se completa siempre (también si falló), para que no se quede a medias en el panel de tareas.
    let _ = emit_loading(&bar, (100.0 - last).max(0.0), None);
    let report = result?;

    if let Some(origin) = origin {
        let state = State::get().await?;
        for world in &report.installed {
            attached_world_data::set_project_id(instance_id, WorldType::Singleplayer, &world.folder, &origin.project_id, &state.pool).await?;
            attached_world_data::set_content_kind(instance_id, WorldType::Singleplayer, &world.folder, &origin.content_kind, &state.pool).await?;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quartz_nbt::{NbtCompound, io::Flavor};
    use std::io::Cursor;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    fn level_dat(name: &str, version: &str) -> Vec<u8> {
        let mut version_tag = NbtCompound::new();
        version_tag.insert("Name", version);
        version_tag.insert("Id", 3465i32);
        let mut data = NbtCompound::new();
        data.insert("LevelName", name);
        data.insert("DataVersion", 3465i32);
        data.insert("Version", version_tag);
        data.insert("Padding", vec![1i8, 2, 3]);
        let mut root = NbtCompound::new();
        root.insert("Data", data);
        let mut out = Vec::new();
        quartz_nbt::io::write_nbt(&mut out, None, &root, Flavor::GzCompressed).unwrap();
        out
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    struct Setup {
        _root: tempfile::TempDir,
        zip: PathBuf,
        instance: PathBuf,
    }

    fn setup(zip_bytes: &[u8]) -> Setup {
        let root = tempfile::tempdir().unwrap();
        let zip = root.path().join("mapa.zip");
        fs::write(&zip, zip_bytes).unwrap();
        let instance = root.path().join("instancia");
        fs::create_dir_all(&instance).unwrap();
        Setup { _root: root, zip, instance }
    }

    fn run(setup: &Setup, selected: &[&str]) -> Result<InstallReport, ImportError> {
        let selected: Vec<String> = selected.iter().map(|s| s.to_string()).collect();
        install(&setup.zip, &setup.instance, &selected, &Limits::default(), &mut |_| {})
    }

    fn saves(setup: &Setup) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(setup.instance.join("saves"))
            .map(|dir| dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Nada debe haberse escrito fuera de `saves` ni quedar restos de trabajo en la instancia.
    fn assert_untouched(setup: &Setup) {
        assert!(saves(setup).is_empty(), "no debe quedar ningún mundo: {:?}", saves(setup));
        let leftovers: Vec<_> = fs::read_dir(&setup.instance).unwrap().map(|e| e.unwrap().file_name()).filter(|n| n != "saves").collect();
        assert!(leftovers.is_empty(), "restos en la instancia: {leftovers:?}");
        assert!(!setup.instance.parent().unwrap().join("evil.txt").exists());
    }

    #[test]
    fn installs_a_world_in_the_root_of_the_zip() {
        let dat = level_dat("Mi castillo", "1.20.1");
        let setup = setup(&make_zip(&[("level.dat", &dat), ("region/r.0.0.mca", b"abc"), ("data/x.dat", b"y")]));
        let report = run(&setup, &[]).unwrap();
        assert_eq!(report.installed.len(), 1);
        assert_eq!(report.installed[0].folder, "Mi castillo", "sin carpeta en el ZIP se usa el nombre del mundo");
        assert_eq!(report.installed[0].game_version.as_deref(), Some("1.20.1"));
        assert!(setup.instance.join("saves/Mi castillo/level.dat").is_file());
        assert_eq!(fs::read(setup.instance.join("saves/Mi castillo/region/r.0.0.mca")).unwrap(), b"abc");
        assert_eq!(fs::read_dir(&setup.instance).unwrap().count(), 1, "solo existe saves: la carpeta de trabajo se limpia");
    }

    #[test]
    fn finds_a_world_inside_subfolders_and_ignores_macos_junk() {
        let dat = level_dat("Parkour", "1.19.4");
        let setup = setup(&make_zip(&[
            ("Pack/Parkour Mapa/level.dat", &dat),
            ("Pack/Parkour Mapa/region/r.0.0.mca", b"abc"),
            ("__MACOSX/Pack/Parkour Mapa/._level.dat", b"basura"),
            ("Pack/LEEME.txt", b"hola"),
        ]));
        let inspection = inspect(&setup.zip).unwrap();
        assert_eq!(inspection.worlds.len(), 1);
        assert_eq!(inspection.worlds[0].path, "Pack/Parkour Mapa");
        assert_eq!(inspection.worlds[0].folder, "Parkour Mapa");
        assert_eq!(inspection.worlds[0].files, 2, "solo lo que está dentro del mundo");
        let report = run(&setup, &[]).unwrap();
        assert_eq!(report.installed[0].folder, "Parkour Mapa");
        assert!(setup.instance.join("saves/Parkour Mapa/region/r.0.0.mca").is_file());
        assert!(!setup.instance.join("saves/Parkour Mapa/LEEME.txt").exists());
    }

    #[test]
    fn several_worlds_need_a_choice_and_install_only_the_chosen_ones() {
        let (a, b) = (level_dat("Uno", "1.20.1"), level_dat("Dos", "1.20.1"));
        let setup = setup(&make_zip(&[("Uno/level.dat", &a), ("Uno/x", b"1"), ("Dos/level.dat", &b), ("Dos/x", b"2")]));
        assert_eq!(inspect(&setup.zip).unwrap().worlds.len(), 2);
        assert_eq!(run(&setup, &[]), Err(ImportError::NeedsSelection));
        assert_untouched(&setup);
        assert_eq!(run(&setup, &["Tres"]), Err(ImportError::UnknownWorld("Tres".into())));
        let report = run(&setup, &["Dos"]).unwrap();
        assert_eq!(saves(&setup), vec!["Dos".to_string()]);
        assert_eq!(report.installed.len(), 1);
        let both = run(&setup, &["Uno", "Dos"]).unwrap();
        assert_eq!(both.installed.iter().map(|w| w.folder.as_str()).collect::<Vec<_>>(), vec!["Uno", "Dos (2)"]);
    }

    #[test]
    fn a_world_inside_another_world_is_not_a_second_world() {
        let (a, b) = (level_dat("Grande", "1.20.1"), level_dat("Falso", "1.20.1"));
        let setup = setup(&make_zip(&[("Grande/level.dat", &a), ("Grande/datapacks/x/level.dat", &b)]));
        assert_eq!(inspect(&setup.zip).unwrap().worlds.len(), 1);
    }

    #[test]
    fn duplicate_names_get_a_suffix_and_never_overwrite() {
        let dat = level_dat("Mapa", "1.20.1");
        let setup = setup(&make_zip(&[("Mapa/level.dat", &dat), ("Mapa/x", b"nuevo")]));
        fs::create_dir_all(setup.instance.join("saves/Mapa")).unwrap();
        fs::write(setup.instance.join("saves/Mapa/x"), b"original").unwrap();
        let first = run(&setup, &[]).unwrap();
        let second = run(&setup, &[]).unwrap();
        assert_eq!(first.installed[0].folder, "Mapa (2)");
        assert_eq!(second.installed[0].folder, "Mapa (3)");
        assert_eq!(fs::read(setup.instance.join("saves/Mapa/x")).unwrap(), b"original");
        assert_eq!(fs::read(setup.instance.join("saves/Mapa (2)/x")).unwrap(), b"nuevo");
    }

    #[test]
    fn a_zip_without_level_dat_is_rejected() {
        let setup = setup(&make_zip(&[("cosas/foto.png", b"png"), ("LEEME.txt", b"hola")]));
        assert_eq!(inspect(&setup.zip), Err(ImportError::NoWorld));
        assert_eq!(run(&setup, &[]), Err(ImportError::NoWorld));
        assert_untouched(&setup);
    }

    #[test]
    fn a_corrupt_level_dat_is_not_a_world() {
        let setup = setup(&make_zip(&[("Mapa/level.dat", b"esto no es gzip ni nbt")]));
        assert_eq!(inspect(&setup.zip), Err(ImportError::NoWorld));
        let mut truncated = level_dat("Roto", "1.20.1");
        truncated.truncate(truncated.len() / 2);
        let setup = self::setup(&make_zip(&[("Mapa/level.dat", &truncated)]));
        assert_eq!(inspect(&setup.zip), Err(ImportError::NoWorld));
    }

    #[test]
    fn zip_slip_is_rejected_in_every_form_and_writes_nothing() {
        let dat = level_dat("Mapa", "1.20.1");
        for evil in [
            "../evil.txt",
            "Mapa/../../evil.txt",
            "..\\evil.txt",
            "Mapa\\..\\..\\evil.txt",
            "/evil.txt",
            "\\evil.txt",
            "C:\\evil.txt",
            "c:evil.txt",
            "\\\\servidor\\recurso\\evil.txt",
            "//servidor/recurso/evil.txt",
            "Mapa/dato.txt:secreto",
            "Mapa/CON",
            "Mapa/nul.txt",
            "Mapa/lpt1",
            "Mapa/espacio ",
            "Mapa/punto.",
            "Mapa/ra\u{1}ro",
        ] {
            let setup = setup(&make_zip(&[("Mapa/level.dat", &dat), (evil, b"malo")]));
            assert!(matches!(inspect(&setup.zip), Err(ImportError::Unsafe(_))), "inspect debería rechazar {evil:?}");
            assert!(matches!(run(&setup, &[]), Err(ImportError::Unsafe(_))), "install debería rechazar {evil:?}");
            assert_untouched(&setup);
        }
    }

    #[test]
    fn symlinks_are_rejected() {
        let dat = level_dat("Mapa", "1.20.1");
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        writer.start_file("Mapa/level.dat", options).unwrap();
        writer.write_all(&dat).unwrap();
        writer.add_symlink("Mapa/enlace", "../../../etc", options).unwrap();
        let setup = setup(&writer.finish().unwrap().into_inner());
        assert!(matches!(run(&setup, &[]), Err(ImportError::Unsafe(_))));
        assert_untouched(&setup);
    }

    #[test]
    fn compression_bombs_and_limits_are_rejected() {
        let dat = level_dat("Mapa", "1.20.1");
        // 20 MB de ceros comprimen más de 200 veces.
        let zeros = vec![0u8; 20 * 1024 * 1024];
        let setup = setup(&make_zip(&[("Mapa/level.dat", &dat), ("Mapa/bomba.bin", &zeros)]));
        assert_eq!(inspect(&setup.zip), Err(ImportError::Suspicious));
        assert_eq!(run(&setup, &[]), Err(ImportError::Suspicious));
        assert_untouched(&setup);

        // Tamaño total.
        let small = Limits { max_total_bytes: 1024, ..Limits::default() };
        let big = vec![7u8; 4096];
        let setup = self::setup(&make_zip(&[("Mapa/level.dat", &dat), ("Mapa/a", &big)]));
        assert_eq!(inspect_with(&setup.zip, &small), Err(ImportError::TooBig));

        // Número de archivos.
        let few = Limits { max_files: 2, ..Limits::default() };
        let setup = self::setup(&make_zip(&[("Mapa/level.dat", &dat), ("Mapa/a", b"1"), ("Mapa/b", b"2")]));
        assert_eq!(inspect_with(&setup.zip, &few), Err(ImportError::TooManyFiles));

        // level.dat descomprimido enorme.
        let tiny_dat = Limits { max_level_dat: 64, ..Limits::default() };
        let setup = self::setup(&make_zip(&[("Mapa/level.dat", &dat)]));
        assert!(inspect_with(&setup.zip, &tiny_dat).is_err());
    }

    #[test]
    fn executables_are_not_copied_and_are_reported() {
        let dat = level_dat("Mapa", "1.20.1");
        let setup = setup(&make_zip(&[
            ("Mapa/level.dat", &dat),
            ("Mapa/region/r.0.0.mca", b"abc"),
            ("Mapa/mod.jar", b"MZ"),
            ("Mapa/instalar.EXE", b"MZ"),
            ("Mapa/tools/a.bat", b"x"),
            ("Mapa/tools/b.cmd", b"x"),
            ("Mapa/tools/c.ps1", b"x"),
            ("Mapa/tools/d.vbs", b"x"),
            ("Mapa/tools/e.dll", b"x"),
            ("Mapa/tools/f.scr", b"x"),
            ("Mapa/tools/g.lnk", b"x"),
            ("Mapa/datapacks/pack.mcfunction", b"say hola"),
        ]));
        let inspection = inspect(&setup.zip).unwrap();
        assert_eq!(inspection.blocked_files.len(), 9);
        let report = run(&setup, &[]).unwrap();
        assert_eq!(report.blocked_files.len(), 9);
        let world = setup.instance.join("saves/Mapa");
        assert!(world.join("region/r.0.0.mca").is_file());
        assert!(world.join("datapacks/pack.mcfunction").is_file(), "los datapacks normales sí se instalan");
        for blocked in ["mod.jar", "instalar.EXE", "tools/a.bat", "tools/e.dll", "tools/g.lnk"] {
            assert!(!world.join(blocked).exists(), "{blocked} no debe copiarse");
        }
    }

    #[test]
    fn a_saves_folder_that_is_a_file_or_escapes_the_instance_is_refused() {
        let dat = level_dat("Mapa", "1.20.1");
        let setup = setup(&make_zip(&[("Mapa/level.dat", &dat)]));
        fs::write(setup.instance.join("saves"), b"no soy una carpeta").unwrap();
        assert!(matches!(run(&setup, &[]), Err(ImportError::Unsafe(_))));
    }

    #[test]
    fn a_failure_halfway_through_leaves_no_partial_world() {
        let dat = level_dat("Mapa", "1.20.1");
        // `a` es un archivo y `a/b` necesita que `a` sea una carpeta: falla después de haber escrito level.dat.
        let setup = setup(&make_zip(&[("Mapa/level.dat", &dat), ("Mapa/a", b"archivo"), ("Mapa/a/b", b"dentro")]));
        assert!(matches!(run(&setup, &[]), Err(ImportError::Io(_))));
        assert_untouched(&setup);
    }

    #[test]
    fn a_truncated_zip_is_rejected() {
        let dat = level_dat("Mapa", "1.20.1");
        let bytes = make_zip(&[("Mapa/level.dat", &dat), ("Mapa/region/r.0.0.mca", &vec![9u8; 50_000])]);
        let setup = setup(&bytes[..bytes.len() - 200]); // rompe el directorio central
        assert_eq!(run(&setup, &[]), Err(ImportError::NotAZip));
        assert_untouched(&setup);
    }

    #[test]
    fn folder_names_are_sanitized() {
        assert_eq!(sanitize_folder_name("Mi: mapa/raro?"), "Mi_ mapa_raro_");
        assert_eq!(sanitize_folder_name("  ..oculto.. "), "oculto");
        assert_eq!(sanitize_folder_name("CON"), "Mapa");
        assert_eq!(sanitize_folder_name(""), "Mapa");
        assert_eq!(sanitize_folder_name(&"a".repeat(100)).chars().count(), 64);
        assert_eq!(sanitize_folder_name("Castillo ñandú"), "Castillo ñandú");
    }

    #[test]
    fn the_nbt_reader_survives_hostile_input_without_allocating() {
        // Una lista que dice tener 2.000 millones de elementos pero no trae datos: debe fallar enseguida.
        let mut hostile = vec![10u8, 0, 0, 10, 0, 4];
        hostile.extend_from_slice(b"Data");
        hostile.extend_from_slice(&[9, 0, 1, b'x', 10, 0x7f, 0xff, 0xff, 0xff]);
        assert!(read_level_info(&hostile).is_none());
        assert!(read_level_info(&[]).is_none());
        assert!(read_level_info(&[1, 2, 3]).is_none());
        // Anidamiento excesivo.
        let mut deep = vec![10u8, 0, 0];
        for _ in 0..200 {
            deep.extend_from_slice(&[10, 0, 1, b'a']);
        }
        assert!(read_level_info(&deep).is_none());
    }
}
