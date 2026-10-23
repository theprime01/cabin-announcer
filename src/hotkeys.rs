//! Global hotkeys that work while the sim has focus (and in VR):
//! Ctrl+Shift+F9 replays the last announcement, F10 stops, F11 pauses.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_F9, VK_F10, VK_F11};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY};

use crate::audio::Audio;

pub const HELP: &[(&str, &str)] = &[("Ctrl+Shift+F9", "Replay the last announcement"), ("Ctrl+Shift+F10", "Stop and skip what's queued"), ("Ctrl+Shift+F11", "Pause or resume all sound")];

const KEYS: [(i32, u16); 3] = [(1, VK_F9.0), (2, VK_F10.0), (3, VK_F11.0)];
const WM_SET_ENABLED: u32 = WM_APP + 1;

pub struct Hotkeys {
    thread: Arc<AtomicU32>,
}

fn register(on: bool) -> bool {
    let mut ok = true;
    for (id, vk) in KEYS {
        unsafe {
            let _ = UnregisterHotKey(HWND::default(), id);
            if on {
                ok &= RegisterHotKey(HWND::default(), id, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, vk as u32).is_ok();
            }
        }
    }
    ok
}

impl Hotkeys {
    /// `on_fail` is told when another program already owns the keys.
    pub fn start(audio: Audio, enabled: bool, on_fail: impl Fn() + Send + 'static) -> Hotkeys {
        let thread = Arc::new(AtomicU32::new(0));
        let t = thread.clone();
        std::thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || {
                let mut msg = MSG::default();
                unsafe {
                    // Creates this thread's message queue before anyone posts to it.
                    let _ = PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_NOREMOVE);
                    t.store(GetCurrentThreadId(), Ordering::SeqCst);
                }
                if !register(enabled) {
                    on_fail();
                }
                while unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) }.as_bool() {
                    match msg.message {
                        WM_HOTKEY => match msg.wParam.0 {
                            1 => audio.replay(),
                            2 => audio.stop(),
                            3 => audio.toggle_pause(),
                            _ => {}
                        },
                        WM_SET_ENABLED => {
                            if !register(msg.wParam.0 != 0) {
                                on_fail();
                            }
                        }
                        _ => {}
                    }
                }
            })
            .expect("spawn hotkey thread");
        Hotkeys { thread }
    }

    pub fn set_enabled(&self, on: bool) {
        let id = self.thread.load(Ordering::SeqCst);
        if id != 0 {
            unsafe {
                let _ = PostThreadMessageW(id, WM_SET_ENABLED, WPARAM(on as usize), LPARAM(0));
            }
        }
    }
}
