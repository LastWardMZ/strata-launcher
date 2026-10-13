//! Avisos «tipo Steam» sobre la esquina superior derecha de la ventana de Minecraft: mensajes de amigos, invitaciones,
//! peticiones de unirse y pistas sobre abrir el mundo a LAN.
//!
//! Igual que el indicador REC, es una ventana de Win32 sin foco que deja pasar el ratón, y solo se ve mientras
//! Minecraft es la ventana en primer plano (si estás en otra, el aviso caduca solo y Strata ya enseña el suyo).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DT_END_ELLIPSIS,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint, FW_BOLD, FW_NORMAL, FillRect, GetDC, GetDeviceCaps,
    GetStockObject, HGDIOBJ, InvalidateRect, LOGPIXELSX, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ReleaseDC, RoundRect, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, HWND_TOPMOST, LWA_ALPHA, LWA_COLORKEY, MA_NOACTIVATE, MSG,
    GetSystemMetrics, PM_REMOVE, PeekMessageW, RegisterClassW, SM_CXSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
    TranslateMessage, WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

/// Medidas a 96 ppp; se escalan con el DPI de la pantalla.
const WIDTH: i32 = 340;
const CARD_HEIGHT: i32 = 80;
const GAP: i32 = 8;
const MARGIN: i32 = 20;
/// Debajo del indicador REC, que ocupa la misma esquina.
const TOP_OFFSET: i32 = 70;
const MAX_VISIBLE: usize = 3;
const LIFETIME: Duration = Duration::from_secs(7);
/// Color transparente de la ventana (magenta, en formato BGR).
const KEY: u32 = 0x00FF_00FF;
const DARK: u32 = 0x0024_1E1B;
const ACCENT: u32 = 0x00C8_B51E;

#[derive(Clone)]
struct Toast {
    title: String,
    body: String,
    born: Instant,
}

static TOASTS: Mutex<Vec<Toast>> = Mutex::new(Vec::new());
static DPI: Mutex<i32> = Mutex::new(96);
static STARTED: AtomicBool = AtomicBool::new(false);
/// Hay una ventana de Minecraft a la que enseñar los avisos.
static HAS_TARGET: AtomicBool = AtomicBool::new(false);

enum Command {
    Target(Option<isize>),
    Wake,
}

static SENDER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();

fn sender() -> &'static mpsc::Sender<Command> {
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        STARTED.store(true, Ordering::Relaxed);
        let _ = std::thread::Builder::new().name("strata-avisos".into()).spawn(move || run(rx));
        tx
    })
}

/// Recorta el texto a `max` caracteres (con «…») para que un mensaje largo no desborde el aviso.
pub fn shorten(text: &str, max: usize) -> String {
    let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= max {
        return clean;
    }
    let mut cut: String = clean.chars().take(max.saturating_sub(1)).collect();
    cut = cut.trim_end().to_string();
    cut.push('…');
    cut
}

/// Alto de la ventana para `count` avisos a `dpi` ppp.
fn stack_height(count: usize, dpi: i32) -> i32 {
    if count == 0 {
        return 0;
    }
    let units = count as i32 * CARD_HEIGHT + (count as i32 - 1) * GAP;
    units * dpi / 96
}

/// Pone un aviso en cola. Si el juego no está en primer plano caduca sin mostrarse.
pub fn push(title: &str, body: &str) {
    {
        let mut toasts = TOASTS.lock().unwrap();
        toasts.insert(0, Toast { title: shorten(title, 40), body: shorten(body, 120), born: Instant::now() });
        toasts.truncate(MAX_VISIBLE * 2);
    }
    let _ = sender().send(Command::Wake);
}

/// Ventana del juego sobre la que se muestran los avisos (`None` si no hay ninguno en marcha).
pub fn set_target(window: Option<isize>) {
    HAS_TARGET.store(window.is_some(), Ordering::Relaxed);
    // Sin avisos pendientes no hace falta arrancar el hilo solo para decirle que no hay ventana.
    if window.is_none() && !STARTED.load(Ordering::Relaxed) {
        return;
    }
    let _ = sender().send(Command::Target(window));
}

/// ¿Hay ahora mismo un Minecraft en marcha donde enseñar avisos?
pub fn has_game() -> bool {
    HAS_TARGET.load(Ordering::Relaxed)
}

/// Solo para pruebas visuales (`STRATA_TOAST_DEBUG=1`): enseña los avisos arriba a la derecha de la pantalla aunque no haya
/// juego en primer plano.
fn debug_always_visible() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var("STRATA_TOAST_DEBUG").is_ok_and(|value| value == "1"))
}

/// Ventana de Minecraft de entre los procesos indicados (la que tiene el foco si es una de ellas).
pub fn game_window_for(pids: &[u32]) -> Option<isize> {
    if pids.is_empty() {
        return None;
    }
    super::win::foreground_pid()
        .filter(|pid| pids.contains(pid))
        .and_then(|pid| super::win::find_window(&[pid]))
        .or_else(|| super::win::find_window(pids))
        .map(|window| window.hwnd)
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_PAINT => {
            // SAFETY: se pinta únicamente sobre la ventana que recibe el mensaje.
            unsafe { paint(hwnd) };
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: llamada estándar al procedimiento por defecto.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn visible_toasts() -> Vec<Toast> {
    let mut toasts = TOASTS.lock().unwrap();
    toasts.retain(|toast| toast.born.elapsed() < LIFETIME);
    toasts.iter().take(MAX_VISIBLE).cloned().collect()
}

unsafe fn font(height: i32, weight: i32) -> windows::Win32::Graphics::Gdi::HFONT {
    // SAFETY: crea una fuente que quien llama libera con `DeleteObject`.
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

unsafe fn paint(hwnd: HWND) {
    let dpi = *DPI.lock().unwrap();
    let scale = |value: i32| value * dpi / 96;
    let toasts = visible_toasts();
    // SAFETY: todo el dibujo ocurre entre `BeginPaint` y `EndPaint`; los objetos GDI creados aquí se liberan antes de salir.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let key = CreateSolidBrush(COLORREF(KEY));
        FillRect(hdc, &client, key);
        let _ = DeleteObject(HGDIOBJ(key.0));

        let accent = CreateSolidBrush(COLORREF(ACCENT));
        let dark = CreateSolidBrush(COLORREF(DARK));
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let title_font = font(scale(15), FW_BOLD.0 as i32);
        let body_font = font(scale(13), FW_NORMAL.0 as i32);
        let fallback = GetStockObject(DEFAULT_GUI_FONT);
        SetBkMode(hdc, TRANSPARENT);

        for (index, toast) in toasts.iter().enumerate() {
            let top = scale(index as i32 * (CARD_HEIGHT + GAP));
            let bottom = top + scale(CARD_HEIGHT);
            let radius = scale(14);
            let old_brush = SelectObject(hdc, HGDIOBJ(accent.0));
            let _ = RoundRect(hdc, 0, top, client.right, bottom, radius, radius);
            SelectObject(hdc, HGDIOBJ(dark.0));
            let _ = RoundRect(hdc, scale(5), top, client.right, bottom, radius, radius);
            SelectObject(hdc, old_brush);

            let left = scale(18);
            let right = client.right - scale(12);
            SetTextColor(hdc, COLORREF(0x00FF_FFFF));
            let old_font = SelectObject(hdc, if title_font.is_invalid() { fallback } else { HGDIOBJ(title_font.0) });
            let mut title_rect = RECT { left, top: top + scale(10), right, bottom: top + scale(32) };
            let mut title: Vec<u16> = toast.title.encode_utf16().collect();
            DrawTextW(hdc, &mut title, &mut title_rect, DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX | DT_LEFT);

            SetTextColor(hdc, COLORREF(0x00D3_CAC5));
            SelectObject(hdc, if body_font.is_invalid() { fallback } else { HGDIOBJ(body_font.0) });
            let mut body_rect = RECT { left, top: top + scale(33), right, bottom: bottom - scale(8) };
            let mut body: Vec<u16> = toast.body.encode_utf16().collect();
            DrawTextW(hdc, &mut body, &mut body_rect, DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX | DT_LEFT);
            SelectObject(hdc, old_font);
        }

        SelectObject(hdc, old_pen);
        for object in [accent.0, dark.0] {
            let _ = DeleteObject(HGDIOBJ(object));
        }
        for object in [title_font, body_font] {
            if !object.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(object.0));
            }
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn system_dpi() -> i32 {
    // SAFETY: consulta de solo lectura del DC de la pantalla, que se libera enseguida.
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return 96;
        }
        let dpi = GetDeviceCaps(Some(hdc), LOGPIXELSX);
        let _ = ReleaseDC(None, hdc);
        if dpi > 0 { dpi } else { 96 }
    }
}

fn run(rx: mpsc::Receiver<Command>) {
    // SAFETY: ventana propia de este hilo; todas las llamadas ocurren aquí y la ventana se destruye al salir.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = w!("StrataGameToasts");
        let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: module.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let dpi = system_dpi();
        *DPI.lock().unwrap() = dpi;
        let width = WIDTH * dpi / 96;
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            class,
            w!("Strata avisos"),
            WS_POPUP,
            0,
            0,
            width,
            stack_height(1, dpi),
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            return;
        };
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(KEY), 242, LWA_COLORKEY | LWA_ALPHA);

        let mut target: Option<isize> = None;
        let mut visible = false;
        let mut last_count = 0usize;
        loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            match rx.recv_timeout(Duration::from_millis(120)) {
                Ok(Command::Target(window)) => target = window,
                Ok(Command::Wake) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let count = visible_toasts().len();
            let debug = debug_always_visible();
            let focused = debug
                || target.is_some_and(|window| {
                    super::win::is_alive(window) && !super::win::is_minimized(window) && super::win::foreground_window() == window
                });
            if focused && count > 0 {
                let height = stack_height(count, dpi);
                if debug {
                    let x = GetSystemMetrics(SM_CXSCREEN) - width - MARGIN * dpi / 96;
                    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, TOP_OFFSET * dpi / 96, width, height, SWP_NOACTIVATE);
                } else if let Some(rect) = target.and_then(super::win::client_rect_on_screen) {
                    let x = (rect.right - width - MARGIN * dpi / 96).max(rect.left);
                    let y = rect.top + TOP_OFFSET * dpi / 96;
                    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE);
                }
                let just_shown = !visible;
                if just_shown {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    visible = true;
                }
                // Solo se repinta cuando cambia lo que hay que enseñar (evita parpadeos).
                if just_shown || count != last_count {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            } else if visible {
                let _ = ShowWindow(hwnd, SW_HIDE);
                visible = false;
            }
            last_count = count;
        }
        let _ = DestroyWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_texts_are_shortened_on_one_line() {
        assert_eq!(shorten("hola", 10), "hola");
        assert_eq!(shorten("  muchos   espacios \n y saltos ", 40), "muchos espacios y saltos");
        let long = "a".repeat(200);
        let short = shorten(&long, 20);
        assert_eq!(short.chars().count(), 20);
        assert!(short.ends_with('…'));
        assert_eq!(shorten("áéíóú ñandú", 5), "áéíó…", "cuenta caracteres, no bytes");
    }

    #[test]
    fn the_stack_grows_with_each_toast_and_scales_with_dpi() {
        assert_eq!(stack_height(0, 96), 0);
        assert_eq!(stack_height(1, 96), CARD_HEIGHT);
        assert_eq!(stack_height(3, 96), 3 * CARD_HEIGHT + 2 * GAP);
        assert_eq!(stack_height(1, 192), 2 * CARD_HEIGHT);
    }

    #[test]
    fn at_most_three_toasts_are_shown_and_old_ones_expire() {
        TOASTS.lock().unwrap().clear();
        {
            let mut toasts = TOASTS.lock().unwrap();
            for index in 0..5 {
                toasts.push(Toast { title: format!("t{index}"), body: String::new(), born: Instant::now() });
            }
            toasts.push(Toast { title: "viejo".into(), body: String::new(), born: Instant::now() - LIFETIME - Duration::from_secs(1) });
        }
        let shown = visible_toasts();
        assert_eq!(shown.len(), MAX_VISIBLE);
        assert!(TOASTS.lock().unwrap().iter().all(|toast| toast.title != "viejo"), "el caducado se descarta");
        TOASTS.lock().unwrap().clear();
    }
}
