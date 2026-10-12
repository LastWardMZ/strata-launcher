//! Tecla global de grabación con `RegisterHotKey`: funciona con Minecraft en primer plano (y con cualquier otra
//! ventana) porque la gestiona el sistema, no la interfaz del launcher. La tecla se registra solo mientras hay un
//! Minecraft lanzado desde Strata, para no quitarle esa tecla al resto del sistema el resto del tiempo.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY, WM_USER};

use super::settings::Hotkey;

const HOTKEY_ID: i32 = 0x5354;
/// «La combinación de teclas ya está registrada por otro programa».
const ERROR_HOTKEY_ALREADY_REGISTERED: i32 = 1409;

enum Command {
    Register { modifiers: u32, vk: u32, reply: mpsc::Sender<Result<(), String>> },
    Unregister,
    Quit,
}

pub struct Hotkeys {
    tx: mpsc::Sender<Command>,
    thread_id: Arc<AtomicU32>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Hotkeys {
    /// Arranca el hilo de mensajes. `on_press` se llama (desde ese hilo) cada vez que se pulsa la combinación.
    pub fn start(on_press: impl Fn() + Send + 'static) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Command>();
        let thread_id = Arc::new(AtomicU32::new(0));
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let id = thread_id.clone();
        let handle = std::thread::Builder::new()
            .name("strata-hotkey".into())
            .spawn(move || {
                // SAFETY: llamadas de Win32 desde el propio hilo; `msg` es válido durante cada llamada.
                unsafe {
                    // Una consulta de mensajes crea la cola del hilo, necesaria para recibir `WM_APP`.
                    let mut msg = MSG::default();
                    let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
                    id.store(GetCurrentThreadId(), Ordering::SeqCst);
                    let _ = ready_tx.send(());
                    let mut registered = false;
                    loop {
                        let result = GetMessageW(&mut msg, None, 0, 0);
                        if result.0 <= 0 {
                            break;
                        }
                        match msg.message {
                            WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_ID => on_press(),
                            WM_APP => {
                                let mut quit = false;
                                while let Ok(command) = rx.try_recv() {
                                    match command {
                                        Command::Register { modifiers, vk, reply } => {
                                            if registered {
                                                let _ = UnregisterHotKey(None, HOTKEY_ID);
                                            }
                                            let outcome = RegisterHotKey(None, HOTKEY_ID, HOT_KEY_MODIFIERS(modifiers) | MOD_NOREPEAT, vk);
                                            registered = outcome.is_ok();
                                            let _ = reply.send(outcome.map_err(|error| {
                                                if error.code().0 & 0xFFFF == ERROR_HOTKEY_ALREADY_REGISTERED {
                                                    "Otra aplicación ya usa esa combinación de teclas".to_string()
                                                } else {
                                                    format!("No se pudo registrar la tecla: {error}")
                                                }
                                            }));
                                        }
                                        Command::Unregister => {
                                            if registered {
                                                let _ = UnregisterHotKey(None, HOTKEY_ID);
                                                registered = false;
                                            }
                                        }
                                        Command::Quit => quit = true,
                                    }
                                }
                                if quit {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if registered {
                        let _ = UnregisterHotKey(None, HOTKEY_ID);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_| "El hilo de teclas no arrancó".to_string())?;
        Ok(Self { tx, thread_id, handle: Some(handle) })
    }

    fn wake(&self) {
        let id = self.thread_id.load(Ordering::SeqCst);
        // SAFETY: el hilo destino existe mientras `self` viva.
        unsafe {
            let _ = PostThreadMessageW(id, WM_APP, WPARAM(0), LPARAM(0));
        }
    }

    pub fn register(&self, hotkey: &Hotkey) -> Result<(), String> {
        let vk = hotkey.validate()?;
        let mut modifiers = 0u32;
        if hotkey.ctrl {
            modifiers |= MOD_CONTROL.0;
        }
        if hotkey.alt {
            modifiers |= MOD_ALT.0;
        }
        if hotkey.shift {
            modifiers |= MOD_SHIFT.0;
        }
        let (reply, answer) = mpsc::channel();
        self.tx.send(Command::Register { modifiers, vk, reply }).map_err(|_| "El hilo de teclas se detuvo".to_string())?;
        self.wake();
        answer.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_| "El hilo de teclas no responde".to_string())?
    }

    pub fn unregister(&self) {
        if self.tx.send(Command::Unregister).is_ok() {
            self.wake();
        }
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        self.wake();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
