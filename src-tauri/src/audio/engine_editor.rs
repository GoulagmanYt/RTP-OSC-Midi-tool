#![allow(deprecated)]

use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
};

use parking_lot::Mutex;
use rack::PluginInstance as _;
use vst::plugin::Plugin;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, IsWindowVisible, KillTimer,
    RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, ShowWindow, CREATESTRUCTW,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HCURSOR, HICON, SW_HIDE, SW_SHOW,
    WM_CLOSE, WM_CREATE, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::types::VstParameter;

use super::{
    engine::AudioEngine,
    runtime_state::{AudioError, EditorWindow, PluginBackend, VST_EDITOR_IDLE_TIMER_MS},
    state_codec::save_vst_state,
};

struct WindowData {
    state: std::sync::Weak<Mutex<Option<EditorWindow>>>,
    app_handle: Option<tauri::AppHandle>,
    plugin: Option<Arc<Mutex<PluginBackend>>>,
    vst_path: Option<PathBuf>,
}

unsafe extern "system" fn vst_window_proc(
    hwnd: HWND,
    msg: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            // SAFETY: Windows sends a valid CREATESTRUCTW pointer during WM_CREATE for this window.
            let create_struct = lparam.0 as *const CREATESTRUCTW;
            // SAFETY: lpCreateParams is the Box<WindowData> pointer passed in CreateWindowExW.
            let data_ptr = unsafe { (*create_struct).lpCreateParams as *mut WindowData };
            // SAFETY: hwnd is valid during WM_CREATE and we store our user data pointer there.
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize) };
            // SAFETY: starting a UI timer on a live window is valid.
            unsafe { SetTimer(Some(hwnd), 1, VST_EDITOR_IDLE_TIMER_MS, None) };
            LRESULT(0)
        }
        WM_TIMER => {
            // SAFETY: timer handler only touches window-local user data for this window.
            unsafe {
                if IsWindowVisible(hwnd).as_bool() {
                    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData;
                    if !ptr.is_null() {
                        if let Some(arc) = (*ptr).state.upgrade() {
                            if let Some(EditorWindow::Vst2 { editor, .. }) = arc.lock().as_mut() {
                                editor.idle();
                            }
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // SAFETY: hwnd is valid while processing WM_CLOSE.
            let _ = unsafe { KillTimer(Some(hwnd), 1) };
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };

            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowData };
            if !ptr.is_null() {
                // SAFETY: ptr comes from our Box<WindowData> stored at WM_CREATE and lives until WM_DESTROY.
                unsafe {
                    if let (Some(plugin), Some(vst_path)) =
                        ((*ptr).plugin.as_ref(), (*ptr).vst_path.as_ref())
                    {
                        save_vst_state(plugin, vst_path);
                    }
                    if let Some(handle) = &(*ptr).app_handle {
                        use tauri::Emitter;
                        let _ = handle.emit("audio:vst-editor-hidden", ());
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = unsafe { KillTimer(Some(hwnd), 1) };
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData };
            if !ptr.is_null() {
                // SAFETY: ptr was allocated with Box::into_raw and is owned by this window.
                let _ = unsafe { Box::from_raw(ptr) };
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, _wparam, lparam) },
    }
}

fn create_vst_window(
    title: &str,
    width: i32,
    height: i32,
    data: WindowData,
) -> Result<HWND, String> {
    // SAFETY: Win32 window class and window creation follow the documented API contract.
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class_name = w!("VSTContainerClass");

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(vst_window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: class_name,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            hCursor: HCURSOR(std::ptr::null_mut()),
            hIcon: HICON(std::ptr::null_mut()),
            ..Default::default()
        };

        let _ = RegisterClassW(&wnd_class);

        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let data_box = Box::new(data);
        let data_ptr = Box::into_raw(data_box);

        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title_wide.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            if width > 0 { width } else { CW_USEDEFAULT },
            if height > 0 { height } else { CW_USEDEFAULT },
            None,
            None,
            Some(HINSTANCE(instance.0)),
            Some(data_ptr as *mut std::ffi::c_void),
        )
        .map_err(|e| {
            let _ = Box::from_raw(data_ptr);
            e.to_string()
        })?;

        if hwnd.0.is_null() {
            let _ = Box::from_raw(data_ptr);
            return Err("Failed to create window".to_string());
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);

        Ok(hwnd)
    }
}

impl AudioEngine {
    pub fn open_vst_ui(&self, app_handle: tauri::AppHandle) -> Result<(), AudioError> {
        let (plugin_arc, editor_window_arc, vst_path) = {
            let mut guard = self.runtime.lock();
            let Some(runtime) = guard.as_mut() else {
                return Err(AudioError::Message("Audio not started".into()));
            };
            (
                runtime.plugin.clone(),
                runtime.editor_window.clone(),
                runtime.vst_path.clone(),
            )
        };

        let weak_editor_window = Arc::downgrade(&editor_window_arc);
        let (tx, rx) = mpsc::channel();
        let app_handle_clone = app_handle.clone();

        app_handle
            .run_on_main_thread(move || {
                let result: Result<(), AudioError> = (|| {
                    if let Some(existing) = editor_window_arc.lock().as_ref() {
                        let hwnd = match existing {
                            EditorWindow::Vst2 { hwnd, .. } | EditorWindow::Vst3 { hwnd, .. } => {
                                hwnd
                            }
                        };
                        unsafe {
                            let _ = SetTimer(Some(*hwnd), 1, VST_EDITOR_IDLE_TIMER_MS, None);
                            let _ = ShowWindow(*hwnd, SW_SHOW);
                            let _ = SetForegroundWindow(*hwnd);
                        }
                        return Ok(());
                    }

                    let mut plugin = plugin_arc.lock();
                    match &mut *plugin {
                        PluginBackend::Vst2 { instance } => {
                            let editor = instance.get_editor();
                            let Some(mut editor) = editor else {
                                return Err(AudioError::Message(
                                    "Plugin does not expose an editor".into(),
                                ));
                            };

                            let (width, height) = editor.size();
                            let win_width = if width > 0 { width + 16 } else { 800 };
                            let win_height = if height > 0 { height + 39 } else { 600 };

                            let window_data = WindowData {
                                state: weak_editor_window,
                                app_handle: Some(app_handle_clone),
                                plugin: Some(plugin_arc.clone()),
                                vst_path: Some(vst_path.clone()),
                            };

                            let hwnd =
                                create_vst_window("VST Editor", win_width, win_height, window_data)
                                    .map_err(AudioError::Message)?;

                            let hwnd_ptr = hwnd.0;
                            if editor.open(hwnd_ptr) {
                                *editor_window_arc.lock() =
                                    Some(EditorWindow::Vst2 { editor, hwnd });
                                Ok(())
                            } else {
                                unsafe {
                                    let _ = DestroyWindow(hwnd);
                                }
                                Err(AudioError::Message("Failed to open VST editor".into()))
                            }
                        }
                        PluginBackend::Vst3 { instance, .. } => {
                            let mut gui = instance.create_gui().map_err(|e| {
                                AudioError::Message(format!("Failed to create VST3 editor: {e}"))
                            })?;
                            let (width, height) = gui.size().map_err(|e| {
                                AudioError::Message(format!("Failed to get VST3 editor size: {e}"))
                            })?;
                            let win_width = if width > 0.0 {
                                width.round() as i32 + 16
                            } else {
                                800
                            };
                            let win_height = if height > 0.0 {
                                height.round() as i32 + 39
                            } else {
                                600
                            };

                            let window_data = WindowData {
                                state: weak_editor_window,
                                app_handle: Some(app_handle_clone),
                                plugin: Some(plugin_arc.clone()),
                                vst_path: Some(vst_path.clone()),
                            };

                            let hwnd = create_vst_window(
                                "VST3 Editor",
                                win_width,
                                win_height,
                                window_data,
                            )
                            .map_err(AudioError::Message)?;

                            if let Err(err) = gui.attach(hwnd.0) {
                                unsafe {
                                    let _ = DestroyWindow(hwnd);
                                }
                                return Err(AudioError::Message(format!(
                                    "Failed to attach VST3 editor: {err}"
                                )));
                            }

                            *editor_window_arc.lock() = Some(EditorWindow::Vst3 { gui, hwnd });
                            Ok(())
                        }
                    }
                })();

                let _ = tx.send(result);
            })
            .map_err(|_| AudioError::Message("Failed to schedule VST UI on main thread".into()))?;

        rx.recv().unwrap_or_else(|_| {
            Err(AudioError::Message(
                "Main thread dropped VST UI result".into(),
            ))
        })
    }

    pub fn close_vst_ui(&self, app_handle: tauri::AppHandle) -> Result<(), AudioError> {
        let (editor_window_arc, plugin_arc, vst_path) = {
            let mut guard = self.runtime.lock();
            let Some(runtime) = guard.as_mut() else {
                return Err(AudioError::Message("Audio not started".into()));
            };
            (
                runtime.editor_window.clone(),
                runtime.plugin.clone(),
                runtime.vst_path.clone(),
            )
        };

        app_handle
            .run_on_main_thread(move || {
                if let Some(editor_win) = editor_window_arc.lock().as_mut() {
                    let hwnd = match editor_win {
                        EditorWindow::Vst2 { hwnd, .. } | EditorWindow::Vst3 { hwnd, .. } => hwnd,
                    };
                    unsafe {
                        let _ = ShowWindow(*hwnd, SW_HIDE);
                    }
                    save_vst_state(&plugin_arc, &vst_path);
                }
            })
            .map_err(|_| {
                AudioError::Message("Failed to schedule VST close on main thread".into())
            })?;

        Ok(())
    }

    pub fn list_vst_parameters(&self) -> Result<Vec<VstParameter>, AudioError> {
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            return Err(AudioError::Message("Audio not started".into()));
        };

        let mut plugin = runtime.plugin.lock();
        match &mut *plugin {
            PluginBackend::Vst3 { instance, .. } => {
                let count = instance.parameter_count();
                let mut params = Vec::with_capacity(count);
                for index in 0..count {
                    let info = instance.parameter_info(index).map_err(|e| {
                        AudioError::Message(format!("VST3 parameter info failed: {}", e))
                    })?;
                    let value = instance.get_parameter(index).map_err(|e| {
                        AudioError::Message(format!("VST3 get parameter failed: {}", e))
                    })?;
                    params.push(VstParameter {
                        index,
                        name: info.name,
                        min: info.min,
                        max: info.max,
                        default: info.default,
                        unit: info.unit,
                        value,
                    });
                }
                Ok(params)
            }
            _ => Err(AudioError::Message(
                "VST3 parameters are only available for VST3 plugins".into(),
            )),
        }
    }

    pub fn set_vst_parameter(&self, index: usize, value: f32) -> Result<(), AudioError> {
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            return Err(AudioError::Message("Audio not started".into()));
        };
        let mut plugin = runtime.plugin.lock();
        match &mut *plugin {
            PluginBackend::Vst3 { instance, .. } => {
                let normalized = value.clamp(0.0, 1.0);
                instance
                    .set_parameter(index, normalized)
                    .map_err(|e| AudioError::Message(format!("VST3 set parameter failed: {}", e)))
            }
            _ => Err(AudioError::Message(
                "VST3 parameters are only available for VST3 plugins".into(),
            )),
        }
    }
}
