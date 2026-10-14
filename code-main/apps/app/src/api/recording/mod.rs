//! Grabación de partidas de Minecraft con una tecla, biblioteca de vídeos y recorte (solo Windows).
//!
//! Tecnología elegida (comparada y comprobada con la documentación oficial):
//! - **Vídeo**: FFmpeg con el filtro `gfxcapture` (Windows.Graphics.Capture): captura solo la ventana del juego por
//!   GPU, sin depender de un mod ni de OBS, y el mismo FFmpeg codifica (NVENC/AMF/Quick Sync/Media Foundation o
//!   x264, probados y con alternativa automática), recorta y genera miniaturas.
//! - **Audio**: FFmpeg no tiene captura de audio de Windows, así que se usa WASAPI (crate `wasapi`, MIT) con
//!   captura por proceso: solo suena el audio de Minecraft, no el del resto del sistema.
//! - **Tecla**: `RegisterHotKey`, que funciona con Minecraft en primer plano y solo se registra mientras hay un
//!   juego lanzado desde Strata.
//! - **Descartado**: OBS/libobs (muy pesado y con distribución compleja), `ddagrab`/`gdigrab` (graban todo el
//!   escritorio o fallan con ventanas aceleradas) y un mod auxiliar (solo valdría para algunos cargadores).

mod args;
mod ffmpeg;
mod library;
mod settings;

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod toast;
#[cfg(windows)]
mod trim;
#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use service::*;
#[cfg(windows)]
pub use toast::{game_window, game_window_for, has_game, push as game_toast, set_target as set_game_window};

/// ¿Mayús+Tab pulsadas ahora mismo?
#[cfg(windows)]
pub(crate) fn shift_tab_down() -> bool {
    win::shift_tab_down()
}

/// ¿Esc pulsada ahora mismo?
#[cfg(windows)]
pub(crate) fn escape_down() -> bool {
    win::escape_down()
}

/// Ventana que tiene el foco del sistema.
#[cfg(windows)]
pub(crate) fn foreground_window() -> isize {
    win::foreground_window()
}

/// Devuelve el foco a una ventana.
#[cfg(windows)]
pub(crate) fn focus_window(window: isize) {
    win::focus(window)
}

/// Zona útil de una ventana en pantalla: `(x, y, ancho, alto)` en píxeles.
#[cfg(windows)]
pub(crate) fn window_area(window: isize) -> Option<(i32, i32, u32, u32)> {
    let rect = win::client_rect_on_screen(window)?;
    Some((rect.left, rect.top, (rect.right - rect.left).max(0) as u32, (rect.bottom - rect.top).max(0) as u32))
}

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
pub use stub::*;

/// Espacio libre (en bytes) del disco de la ruta, si se puede saber.
#[cfg(windows)]
pub(crate) fn free_space(path: &std::path::Path) -> Option<u64> {
    win::free_space(path)
}

#[cfg(not(windows))]
pub(crate) fn free_space(_path: &std::path::Path) -> Option<u64> {
    None
}
