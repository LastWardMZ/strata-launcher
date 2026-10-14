//! En sistemas que no son Windows la grabación no está disponible: los comandos existen para que la interfaz
//! pueda preguntar y mostrar un aviso claro, pero no hacen nada más.

use serde_json::Value;

const UNSUPPORTED: &str = "La grabación de partidas solo está disponible en Windows";

#[derive(Default)]
pub struct RecordingState;

impl RecordingState {
    pub fn close(&self) {}
}

pub fn start_in_background(_app: tauri::AppHandle) {}

#[tauri::command]
pub async fn recording_support() -> bool {
    false
}

#[tauri::command]
pub async fn recording_status() -> Result<Value, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_set_settings(_settings: Value) -> Result<Value, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_install_ffmpeg() -> Result<Value, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_remove_ffmpeg() -> Result<Value, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_reprobe_encoders() -> Result<Value, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_check_conflicts(_hotkey: Value) -> Result<Value, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_toggle() -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_list() -> Result<Value, String> {
    Ok(Value::Array(Vec::new()))
}

#[tauri::command]
pub async fn recording_thumbnail(_path: String) -> Result<Option<String>, String> {
    Ok(None)
}

#[tauri::command]
pub async fn recording_delete(_path: String) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_reveal(_path: String) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_allow_playback(_path: String) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_trim_start(_path: String, _start: f64, _end: f64, _precise: bool, _gif: Option<bool>, _speed: Option<f64>, _mute: Option<bool>) -> Result<String, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_merge_start(_paths: Vec<String>) -> Result<String, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub async fn recording_trim_cancel(_job: String) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

/// Avisos sobre el juego: solo existen en Windows.
pub fn game_toast(_title: &str, _body: &str) {}
pub fn set_game_window(_window: Option<isize>) {}
pub fn has_game() -> bool {
    false
}
pub fn game_window_for(_pids: &[u32]) -> Option<isize> {
    None
}

pub fn game_window() -> Option<isize> {
    None
}

pub(crate) fn shift_tab_down() -> bool {
    false
}

pub(crate) fn foreground_window() -> isize {
    0
}

pub(crate) fn focus_window(_window: isize) {}

pub(crate) fn window_area(_window: isize) -> Option<(i32, i32, u32, u32)> {
    None
}

pub(crate) fn escape_down() -> bool {
    false
}
