//! Panel de amigos sobre el juego (Mayús+Tab), como la superposición de Steam.
//!
//! Es una ventana propia de Strata (sin marco y transparente) que se coloca encima de la ventana de Minecraft y carga la
//! interfaz mínima de `overlay/` con la lista de amigos, el chat y las invitaciones. Se mira el teclado desde fuera
//! (sin ganchos ni inyección en el juego): la pulsación sigue llegando a Minecraft, donde Mayús+Tab no hace nada, y solo se
//! actúa si el juego es la ventana en primer plano, así que en el resto de programas no pasa nada.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "overlay";

/// Ventana del juego sobre la que se abrió el panel: a ella vuelve el foco al cerrarlo.
static LAST_GAME: AtomicIsize = AtomicIsize::new(0);

/// Solo para pruebas (`STRATA_OVERLAY_DEBUG=1`): cualquier ventana en primer plano hace de «juego».
fn debug_any_window() -> bool {
    std::env::var("STRATA_OVERLAY_DEBUG").is_ok_and(|value| value == "1")
}

pub fn start_in_background(app: tauri::AppHandle) {
    let _ = std::thread::Builder::new().name("strata-panel-amigos".into()).spawn(move || {
        let (mut shift_tab_was_down, mut escape_was_down) = (false, false);
        loop {
            std::thread::sleep(Duration::from_millis(40));
            let shift_tab = super::recording::shift_tab_down();
            let escape = super::recording::escape_down();
            let shift_tab_pressed = shift_tab && !shift_tab_was_down;
            let escape_pressed = escape && !escape_was_down;
            (shift_tab_was_down, escape_was_down) = (shift_tab, escape);
            if !shift_tab_pressed && !escape_pressed {
                continue;
            }
            // Con el panel abierto, Mayús+Tab y Esc lo cierran. Se atiende aquí (y no en la página) para que funcione
            // aunque el teclado no esté dentro del contenido web.
            if app.get_webview_window(LABEL).is_some_and(|window| window.is_visible().unwrap_or(false)) {
                hide_panel(&app, true);
                continue;
            }
            if !shift_tab_pressed {
                continue;
            }
            let foreground = super::recording::foreground_window();
            let game = if debug_any_window() { Some(foreground) } else { super::recording::game_window() };
            if foreground == 0 || game != Some(foreground) {
                continue;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = show(&app, foreground).await {
                    tracing::warn!("No se pudo abrir el panel de amigos: {error}");
                }
            });
        }
    });
}

/// Abre (o recoloca) el panel sobre el área de la ventana del juego y le da el foco para poder escribir.
async fn show(app: &tauri::AppHandle, game: isize) -> Result<(), String> {
    let (x, y, width, height) = super::recording::window_area(game).ok_or("No se encuentra la ventana del juego")?;
    LAST_GAME.store(game, Ordering::Relaxed);
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?overlay=1".into()))
            .title("Strata · Amigos")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .focused(false)
            .build()
            .map_err(|e| e.to_string())?,
    };
    window.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    window.set_size(PhysicalSize::new(width, height)).map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();
    let _ = window.emit("overlay-shown", ());
    Ok(())
}

/// Cierra el panel. Con `refocus` (por defecto) el foco vuelve al juego; sin él, solo se oculta (p. ej. si el usuario ya ha
/// pulsado en otra ventana).
#[tauri::command]
pub async fn overlay_hide<R: Runtime>(app: tauri::AppHandle<R>, refocus: Option<bool>) -> Result<(), String> {
    hide_panel(&app, refocus.unwrap_or(true));
    Ok(())
}

fn hide_panel<R: Runtime>(app: &tauri::AppHandle<R>, refocus: bool) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
    let game = LAST_GAME.load(Ordering::Relaxed);
    if refocus && game != 0 {
        super::recording::focus_window(game);
    }
}
