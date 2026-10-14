//! Utilidades de Windows para la grabación: encontrar la ventana de Minecraft y consultar el sistema.

use std::path::Path;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_SHIFT, VK_TAB};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, PCWSTR};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameWindow {
    pub hwnd: isize,
    pub pid: u32,
    pub width: u32,
    pub height: u32,
}

fn hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

pub fn client_size(window: isize) -> Option<(u32, u32)> {
    let mut rect = RECT::default();
    // SAFETY: `rect` es válido y `window` solo se consulta; si el identificador ya no existe, la llamada falla.
    unsafe { GetClientRect(hwnd(window), &mut rect).ok()? };
    let (w, h) = ((rect.right - rect.left).max(0) as u32, (rect.bottom - rect.top).max(0) as u32);
    (w > 0 && h > 0).then_some((w, h))
}

/// Área útil (sin barra de título ni bordes) de la ventana, en coordenadas de pantalla.
pub fn client_rect_on_screen(window: isize) -> Option<RECT> {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: igual que en `client_size`.
    unsafe {
        GetClientRect(hwnd(window), &mut rect).ok()?;
        if !ClientToScreen(hwnd(window), &mut origin).as_bool() {
            return None;
        }
    }
    Some(RECT { left: origin.x, top: origin.y, right: origin.x + rect.right, bottom: origin.y + rect.bottom })
}

/// ¿Están pulsadas Mayús y Tab a la vez? Solo mira el teclado: la pulsación sigue llegando al juego.
pub fn shift_tab_down() -> bool {
    let down = |key: i32| {
        // SAFETY: consulta de solo lectura del estado del teclado.
        let state = unsafe { GetAsyncKeyState(key) };
        (state as u16 & 0x8000) != 0
    };
    down(VK_SHIFT.0 as i32) && down(VK_TAB.0 as i32)
}

/// ¿Está pulsada la tecla Esc?
pub fn escape_down() -> bool {
    // SAFETY: consulta de solo lectura del estado del teclado.
    let state = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) };
    (state as u16 & 0x8000) != 0
}

/// Devuelve el foco a una ventana (la restaura si estaba minimizada).
pub fn focus(window: isize) {
    // SAFETY: si la ventana ya no existe las llamadas fallan sin más.
    unsafe {
        if IsIconic(hwnd(window)).as_bool() {
            let _ = ShowWindow(hwnd(window), SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd(window));
    }
}

pub fn is_alive(window: isize) -> bool {
    // SAFETY: consulta de solo lectura.
    unsafe { IsWindow(Some(hwnd(window))).as_bool() }
}

pub fn is_minimized(window: isize) -> bool {
    // SAFETY: consulta de solo lectura.
    unsafe { IsIconic(hwnd(window)).as_bool() }
}

/// Ventana que tiene el foco (0 si ninguna).
pub fn foreground_window() -> isize {
    // SAFETY: consulta de solo lectura.
    unsafe { GetForegroundWindow().0 as isize }
}

/// Proceso dueño de la ventana que tiene el foco.
pub fn foreground_pid() -> Option<u32> {
    // SAFETY: consultas de solo lectura.
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

struct Search<'a> {
    pids: &'a [u32],
    best: Option<(GameWindow, bool)>,
}

unsafe extern "system" fn on_window(window: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` es el puntero a `Search` que pasa `find_window`, vivo durante toda la enumeración.
    let search = unsafe { &mut *(lparam.0 as *mut Search) };
    // SAFETY: consultas de solo lectura sobre la ventana recibida.
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if !search.pids.contains(&pid) {
            return BOOL(1);
        }
        let Some((width, height)) = client_size(window.0 as isize) else { return BOOL(1) };
        let mut class = [0u16; 64];
        let len = GetClassNameW(window, &mut class).max(0) as usize;
        // Minecraft usa GLFW: su ventana es de la clase «GLFW30». Se prefiere sobre cualquier otra del proceso.
        let is_glfw = String::from_utf16_lossy(&class[..len]).starts_with("GLFW");
        let candidate = GameWindow { hwnd: window.0 as isize, pid, width, height };
        let better = match &search.best {
            None => true,
            Some((best, best_glfw)) => (is_glfw, width * height) > (*best_glfw, best.width * best.height),
        };
        if better {
            search.best = Some((candidate, is_glfw));
        }
    }
    BOOL(1)
}

/// Ventana principal visible de alguno de los procesos indicados (la de mayor tamaño; GLFW tiene prioridad).
pub fn find_window(pids: &[u32]) -> Option<GameWindow> {
    let mut search = Search { pids, best: None };
    // SAFETY: el puntero a `search` solo se usa dentro de esta llamada síncrona.
    unsafe {
        let _ = EnumWindows(Some(on_window), LPARAM(&mut search as *mut Search as isize));
    }
    search.best.map(|(window, _)| window)
}

/// Espacio libre (en bytes) del disco que contiene la ruta.
pub fn free_space(path: &Path) -> Option<u64> {
    // Se sube hasta una carpeta que exista: la de destino puede no estar creada todavía.
    let mut existing = path;
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let wide: Vec<u16> = existing.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let mut free = 0u64;
    // SAFETY: `wide` termina en nulo y `free` es válido durante la llamada.
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None).ok()? };
    Some(free)
}
