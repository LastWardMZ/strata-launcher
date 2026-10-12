//! Indicador «● REC mm:ss» sobre la esquina de la ventana del juego mientras se graba.
//!
//! Es una ventana de Win32 pequeña, sin foco, que deja pasar el ratón y solo se ve cuando Minecraft está en primer
//! plano. No sale en el vídeo porque la captura es de la ventana del juego, no del escritorio.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    DT_LEFT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, Ellipse, EndPaint, FW_BOLD, FillRect, GetStockObject, HGDIOBJ,
    InvalidateRect, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE, MSG, PM_REMOVE,
    PeekMessageW, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
    TranslateMessage, WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

const WIDTH: i32 = 170;
const HEIGHT: i32 = 40;
/// Separación respecto al borde de la ventana del juego.
const MARGIN: i32 = 20;
static ELAPSED_SECONDS: AtomicU64 = AtomicU64::new(0);

enum Command {
    Show { target: isize, since: Instant },
    Hide,
    Quit,
}

pub struct Overlay {
    tx: mpsc::Sender<Command>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Overlay {
    pub fn start() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new().name("strata-indicador".into()).spawn(move || run(rx)).map_err(|e| e.to_string())?;
        Ok(Self { tx, handle: Some(handle) })
    }

    /// Muestra el indicador sobre `target` (ventana del juego) contando desde `since`.
    pub fn show(&self, target: isize, since: Instant) {
        let _ = self.tx.send(Command::Show { target, since });
    }

    pub fn hide(&self) {
        let _ = self.tx.send(Command::Hide);
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
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

unsafe fn paint(hwnd: HWND) {
    // SAFETY: todo el dibujo ocurre entre `BeginPaint` y `EndPaint` de esta ventana; los objetos GDI creados aquí se
    // liberan antes de salir.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let background = CreateSolidBrush(COLORREF(0x0024_1E1B));
        FillRect(hdc, &rect, background);
        let _ = DeleteObject(HGDIOBJ(background.0));
        let red = CreateSolidBrush(COLORREF(0x0032_32E6));
        let old_brush = SelectObject(hdc, HGDIOBJ(red.0));
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = Ellipse(hdc, 14, 12, 30, 28);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(red.0));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(0x00FF_FFFF));
        let font = CreateFontW(
            -22,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let old_font = SelectObject(hdc, if font.is_invalid() { GetStockObject(DEFAULT_GUI_FONT) } else { HGDIOBJ(font.0) });
        let seconds = ELAPSED_SECONDS.load(Ordering::Relaxed);
        let mut text: Vec<u16> = format!("REC  {:02}:{:02}", seconds / 60, seconds % 60).encode_utf16().collect();
        rect.left = 38;
        DrawTextW(hdc, &mut text, &mut rect, DT_SINGLELINE | DT_VCENTER | DT_LEFT);
        SelectObject(hdc, old_font);
        if !font.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn run(rx: mpsc::Receiver<Command>) {
    // SAFETY: ventana propia de este hilo; todas las llamadas ocurren aquí y la ventana se destruye al salir.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = w!("StrataRecIndicator");
        let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: module.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            class,
            w!("Strata REC"),
            WS_POPUP,
            0,
            0,
            WIDTH,
            HEIGHT,
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            return;
        };
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 220, LWA_ALPHA);
        let region = CreateRoundRectRgn(0, 0, WIDTH + 1, HEIGHT + 1, 18, 18);
        SetWindowRgn(hwnd, Some(region), true);

        let mut target: Option<(isize, Instant)> = None;
        let mut visible = false;
        loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(Command::Show { target: window, since }) => target = Some((window, since)),
                Ok(Command::Hide) => target = None,
                Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let should_show = target.is_some_and(|(window, _)| {
                super::win::is_alive(window) && !super::win::is_minimized(window) && super::win::foreground_window() == window
            });
            match (should_show, target) {
                (true, Some((window, since))) => {
                    // Esquina superior derecha del área de juego (sin la barra de título).
                    if let Some(rect) = super::win::client_rect_on_screen(window) {
                        let x = (rect.right - WIDTH - MARGIN).max(rect.left);
                        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, rect.top + MARGIN, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
                    }
                    ELAPSED_SECONDS.store(since.elapsed().as_secs(), Ordering::Relaxed);
                    if !visible {
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        visible = true;
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                _ if visible => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    visible = false;
                }
                _ => {}
            }
        }
        let _ = DestroyWindow(hwnd);
    }
}
