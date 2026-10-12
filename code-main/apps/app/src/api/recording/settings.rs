//! Ajustes de grabación: se guardan en `recording/settings.json` dentro de la carpeta de datos de la app
//! (así no hace falta tocar el esquema de la base de datos del launcher) y se validan siempre al leerlos.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const QUALITIES: [&str; 3] = ["low", "medium", "high"];
pub const RESOLUTIONS: [&str; 3] = ["original", "1080p", "720p"];
pub const FORMATS: [&str; 2] = ["mp4", "mkv"];
pub const ENCODER_MODES: [&str; 3] = ["auto", "hardware", "software"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Código de tecla de `KeyboardEvent.code` (`F9`, `KeyR`, `Digit1`, `Numpad0`, `PrintScreen`…).
    pub key: String,
}

impl Default for Hotkey {
    fn default() -> Self {
        // F9 no lo usa Minecraft en ninguna versión por defecto (F1, F2, F3, F5 y F11 sí).
        Self { ctrl: false, alt: false, shift: false, key: "F9".into() }
    }
}

impl Hotkey {
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Mayús".to_string());
        }
        parts.push(key_label(&self.key));
        parts.join(" + ")
    }

    /// Una tecla sin modificadores bloquearía esa tecla en todo el sistema mientras juegas: solo se permite
    /// en teclas que no se usan para escribir.
    pub fn validate(&self) -> Result<u32, String> {
        let vk = vk_from_code(&self.key).ok_or_else(|| format!("La tecla «{}» no está soportada", self.key))?;
        let has_modifier = self.ctrl || self.alt;
        if !has_modifier && !is_safe_alone(&self.key) {
            return Err(
                "Esa tecla sola se usa para escribir. Usa una tecla de función (F1–F24), Pausa, Impr Pant, o combínala con Ctrl o Alt."
                    .into(),
            );
        }
        Ok(vk)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RecordingSettings {
    pub enabled: bool,
    pub hotkey: Hotkey,
    pub fps: u32,
    /// `low`, `medium` o `high`.
    pub quality: String,
    /// `original`, `1080p` o `720p` (altura máxima; nunca se amplía).
    pub resolution: String,
    /// `mp4` o `mkv`.
    pub format: String,
    pub game_audio: bool,
    pub microphone: bool,
    /// `auto` (hardware si funciona, si no software), `hardware` o `software`.
    pub encoder: String,
    /// Carpeta de destino personalizada. Vacío = `recordings` dentro de cada instancia.
    pub folder: Option<String>,
    /// Muestra un indicador «REC» sobre la ventana del juego (no sale en el vídeo).
    pub indicator: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            hotkey: Hotkey::default(),
            fps: 30,
            quality: "medium".into(),
            resolution: "original".into(),
            format: "mp4".into(),
            game_audio: true,
            microphone: false,
            encoder: "auto".into(),
            folder: None,
            indicator: true,
        }
    }
}

impl RecordingSettings {
    pub fn validate(&self) -> Result<(), String> {
        if ![30, 60].contains(&self.fps) {
            return Err("Los FPS deben ser 30 o 60".into());
        }
        if !QUALITIES.contains(&self.quality.as_str()) {
            return Err("Calidad no válida".into());
        }
        if !RESOLUTIONS.contains(&self.resolution.as_str()) {
            return Err("Resolución no válida".into());
        }
        if !FORMATS.contains(&self.format.as_str()) {
            return Err("Formato no válido".into());
        }
        if !ENCODER_MODES.contains(&self.encoder.as_str()) {
            return Err("Modo de codificación no válido".into());
        }
        self.hotkey.validate()?;
        if let Some(folder) = &self.folder {
            validate_folder(folder)?;
        }
        Ok(())
    }

    /// Carpeta personalizada ya validada (o `None`).
    pub fn custom_folder(&self) -> Option<PathBuf> {
        self.folder.as_ref().filter(|f| !f.trim().is_empty()).map(PathBuf::from)
    }
}

/// La carpeta de destino debe ser una ruta absoluta normal: sin `..`, sin rutas de dispositivo ni UNC.
pub fn validate_folder(folder: &str) -> Result<(), String> {
    let folder = folder.trim();
    if folder.is_empty() {
        return Ok(());
    }
    let path = Path::new(folder);
    if !path.is_absolute() {
        return Err("La carpeta de destino debe ser una ruta completa".into());
    }
    if folder.starts_with(r"\\") || folder.starts_with("//") {
        return Err("No se admiten rutas de red ni de dispositivo".into());
    }
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("La carpeta de destino no puede contener «..»".into());
    }
    if folder.len() > 240 {
        return Err("La ruta de la carpeta es demasiado larga".into());
    }
    Ok(())
}

pub fn load(dir: &Path) -> RecordingSettings {
    let mut settings: RecordingSettings = std::fs::read(dir.join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if settings.validate().is_err() {
        // Un archivo editado a mano o de otra versión nunca debe dejar la grabación inutilizable.
        let enabled = settings.enabled;
        settings = RecordingSettings { enabled, ..RecordingSettings::default() };
    }
    settings
}

pub fn save(dir: &Path, settings: &RecordingSettings) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = dir.join("settings.json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join("settings.json")).map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ teclas

/// Código virtual de Windows (VK_*) de un `KeyboardEvent.code`.
pub fn vk_from_code(code: &str) -> Option<u32> {
    if let Some(n) = code.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then(|| 0x70 + n - 1);
    }
    if let Some(letter) = code.strip_prefix("Key") {
        let c = letter.chars().next()?;
        return (letter.len() == 1 && c.is_ascii_uppercase()).then_some(c as u32);
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        let c = digit.chars().next()?;
        return (digit.len() == 1 && c.is_ascii_digit()).then_some(c as u32);
    }
    if let Some(n) = code.strip_prefix("Numpad") {
        if let Ok(d) = n.parse::<u32>() {
            return (d <= 9).then(|| 0x60 + d);
        }
        return match n {
            "Multiply" => Some(0x6A),
            "Add" => Some(0x6B),
            "Subtract" => Some(0x6D),
            "Decimal" => Some(0x6E),
            "Divide" => Some(0x6F),
            _ => None,
        };
    }
    Some(match code {
        "Pause" => 0x13,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "PrintScreen" => 0x2C,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "ScrollLock" => 0x91,
        "Semicolon" => 0xBA,
        "Equal" => 0xBB,
        "Comma" => 0xBC,
        "Minus" => 0xBD,
        "Period" => 0xBE,
        "Slash" => 0xBF,
        "Backquote" => 0xC0,
        "BracketLeft" => 0xDB,
        "Backslash" => 0xDC,
        "BracketRight" => 0xDD,
        "Quote" => 0xDE,
        _ => return None,
    })
}

fn is_safe_alone(code: &str) -> bool {
    code.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| (1..=24).contains(&n))
        || matches!(code, "Pause" | "PrintScreen" | "ScrollLock")
}

pub fn key_label(code: &str) -> String {
    if let Some(letter) = code.strip_prefix("Key") {
        return letter.to_string();
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        return digit.to_string();
    }
    match code {
        "PrintScreen" => "Impr Pant".into(),
        "Pause" => "Pausa".into(),
        "ScrollLock" => "Bloq Despl".into(),
        "PageUp" => "Re Pág".into(),
        "PageDown" => "Av Pág".into(),
        "ArrowLeft" => "←".into(),
        "ArrowRight" => "→".into(),
        "ArrowUp" => "↑".into(),
        "ArrowDown" => "↓".into(),
        other => other.replace("Numpad", "Num ").replace("Backquote", "`"),
    }
}

// ------------------------------------------------------------------ conflictos con Minecraft

/// Nombre de la tecla en `options.txt` de Minecraft (`key.keyboard.<nombre>`).
pub fn minecraft_key_name(code: &str) -> Option<String> {
    if code.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| (1..=24).contains(&n)) {
        return Some(code.to_lowercase());
    }
    if let Some(letter) = code.strip_prefix("Key") {
        return Some(letter.to_lowercase());
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        return Some(digit.to_string());
    }
    if let Some(n) = code.strip_prefix("Numpad") {
        return Some(match n {
            "Multiply" => "keypad.multiply".into(),
            "Add" => "keypad.add".into(),
            "Subtract" => "keypad.subtract".into(),
            "Decimal" => "keypad.decimal".into(),
            "Divide" => "keypad.divide".into(),
            digit => format!("keypad.{digit}"),
        });
    }
    Some(
        match code {
            "Pause" => "pause",
            "PageUp" => "page.up",
            "PageDown" => "page.down",
            "End" => "end",
            "Home" => "home",
            "ArrowLeft" => "left",
            "ArrowUp" => "up",
            "ArrowRight" => "right",
            "ArrowDown" => "down",
            "PrintScreen" => "print.screen",
            "Insert" => "insert",
            "Delete" => "delete",
            "ScrollLock" => "scroll.lock",
            "Semicolon" => "semicolon",
            "Equal" => "equal",
            "Comma" => "comma",
            "Minus" => "minus",
            "Period" => "period",
            "Slash" => "slash",
            "Backquote" => "grave.accent",
            "BracketLeft" => "left.bracket",
            "Backslash" => "backslash",
            "BracketRight" => "right.bracket",
            "Quote" => "apostrophe",
            _ => return None,
        }
        .to_string(),
    )
}

/// Teclas con función propia en Minecraft aunque `options.txt` no las nombre (valores por defecto).
pub fn minecraft_default_binding(code: &str) -> Option<&'static str> {
    match code {
        "F1" => Some("Ocultar la interfaz (F1)"),
        "F2" => Some("Captura de pantalla (F2)"),
        "F3" => Some("Pantalla de depuración (F3)"),
        "F5" => Some("Cambiar la cámara (F5)"),
        "F11" => Some("Pantalla completa (F11)"),
        _ => None,
    }
}

/// Nombres amigables de los atajos de Minecraft más habituales (`key_key.<id>` en `options.txt`).
fn friendly_binding(id: &str) -> String {
    let known = [
        ("key.screenshot", "Captura de pantalla"),
        ("key.fullscreen", "Pantalla completa"),
        ("key.togglePerspective", "Cambiar la cámara"),
        ("key.smoothCamera", "Cámara cinematográfica"),
        ("key.saveToolbarActivator", "Guardar barra de objetos"),
        ("key.loadToolbarActivator", "Cargar barra de objetos"),
        ("key.spectatorOutlines", "Contornos de espectador"),
        ("key.advancements", "Logros"),
        ("key.chat", "Chat"),
        ("key.command", "Comando"),
        ("key.playerlist", "Lista de jugadores"),
        ("key.inventory", "Inventario"),
        ("key.drop", "Soltar objeto"),
        ("key.swapOffhand", "Cambiar mano"),
    ];
    known.iter().find(|(key, _)| *key == id).map_or_else(|| id.to_string(), |(_, name)| (*name).to_string())
}

/// Atajos de Minecraft (de un `options.txt`) que ya usan esa tecla.
pub fn scan_options_conflicts(options_text: &str, key_name: &str) -> Vec<String> {
    let wanted = format!("key.keyboard.{key_name}");
    options_text
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            let id = name.strip_prefix("key_")?;
            let key = value.split(':').next()?.trim();
            (key == wanted).then(|| friendly_binding(id))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_survive_a_roundtrip() {
        let settings = RecordingSettings::default();
        settings.validate().unwrap();
        let json = serde_json::to_string(&settings).unwrap();
        let back: RecordingSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
        assert!(!settings.enabled, "la grabación empieza desactivada");
        assert_eq!(settings.hotkey.label(), "F9");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let partial: RecordingSettings = serde_json::from_str(r#"{"enabled":true,"fps":60}"#).unwrap();
        assert!(partial.enabled);
        assert_eq!(partial.fps, 60);
        assert_eq!(partial.quality, "medium");
        partial.validate().unwrap();
    }

    #[test]
    fn invalid_values_are_rejected() {
        let mut s = RecordingSettings::default();
        s.fps = 45;
        assert!(s.validate().is_err());
        let mut s = RecordingSettings::default();
        s.quality = "ultra".into();
        assert!(s.validate().is_err());
        let mut s = RecordingSettings::default();
        s.format = "avi".into();
        assert!(s.validate().is_err());
        let mut s = RecordingSettings::default();
        s.hotkey.key = "Nope".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn plain_typing_keys_need_a_modifier() {
        let mut s = RecordingSettings::default();
        s.hotkey = Hotkey { ctrl: false, alt: false, shift: false, key: "KeyR".into() };
        assert!(s.validate().is_err(), "la R sola bloquearía escribir");
        s.hotkey.shift = true;
        assert!(s.validate().is_err(), "Mayús no basta");
        s.hotkey = Hotkey { ctrl: true, alt: false, shift: false, key: "KeyR".into() };
        s.validate().unwrap();
        s.hotkey = Hotkey { ctrl: false, alt: false, shift: false, key: "F10".into() };
        s.validate().unwrap();
        s.hotkey = Hotkey { ctrl: false, alt: false, shift: false, key: "PrintScreen".into() };
        s.validate().unwrap();
    }

    #[test]
    fn folders_are_validated() {
        assert!(validate_folder("").is_ok());
        assert!(validate_folder(r"C:\Users\Erik\Videos\Strata").is_ok());
        assert!(validate_folder("relativa/ruta").is_err());
        assert!(validate_folder(r"C:\Users\..\Windows").is_err());
        assert!(validate_folder(r"\\servidor\recurso").is_err());
        assert!(validate_folder(r"\\.\pipe\x").is_err());
    }

    #[test]
    fn invalid_files_do_not_break_loading() {
        let dir = std::env::temp_dir().join(format!("strata-rec-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir), RecordingSettings::default(), "sin archivo: valores por defecto");
        std::fs::write(dir.join("settings.json"), b"{ esto no es json").unwrap();
        assert_eq!(load(&dir), RecordingSettings::default());
        std::fs::write(dir.join("settings.json"), br#"{"enabled":true,"fps":999}"#).unwrap();
        let loaded = load(&dir);
        assert!(loaded.enabled, "se conserva el interruptor");
        assert_eq!(loaded.fps, 30, "el valor inválido se restaura");
        let mut s = RecordingSettings::default();
        s.fps = 60;
        s.enabled = true;
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir), s);
    }

    #[test]
    fn key_codes_map_to_virtual_keys_and_minecraft_names() {
        assert_eq!(vk_from_code("F9"), Some(0x78));
        assert_eq!(vk_from_code("F24"), Some(0x87));
        assert_eq!(vk_from_code("F25"), None);
        assert_eq!(vk_from_code("KeyR"), Some('R' as u32));
        assert_eq!(vk_from_code("Digit1"), Some('1' as u32));
        assert_eq!(vk_from_code("Numpad5"), Some(0x65));
        assert_eq!(vk_from_code("Pause"), Some(0x13));
        assert_eq!(vk_from_code("Keyr"), None);
        assert_eq!(minecraft_key_name("F9").as_deref(), Some("f9"));
        assert_eq!(minecraft_key_name("KeyR").as_deref(), Some("r"));
        assert_eq!(minecraft_key_name("Numpad3").as_deref(), Some("keypad.3"));
        assert_eq!(minecraft_key_name("PageUp").as_deref(), Some("page.up"));
        assert_eq!(minecraft_default_binding("F2"), Some("Captura de pantalla (F2)"));
        assert_eq!(minecraft_default_binding("F9"), None);
    }

    #[test]
    fn conflicts_are_read_from_minecraft_options() {
        let options = "version:3955\nkey_key.screenshot:key.keyboard.f2\nkey_key.togglePerspective:key.keyboard.f9\nkey_key.drop:key.keyboard.q\nkey_key.custom:key.keyboard.f9:key.keyboard.left.shift\nfov:0.5\n";
        assert_eq!(scan_options_conflicts(options, "f9"), vec!["Cambiar la cámara", "key.custom"]);
        assert!(scan_options_conflicts(options, "f10").is_empty());
        assert_eq!(scan_options_conflicts(options, "f2"), vec!["Captura de pantalla"]);
    }
}
