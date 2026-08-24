#![allow(deprecated)]

use std::sync::{mpsc, Arc};

use parking_lot::Mutex;
use vst::plugin::Plugin;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    IsWindowVisible, KillTimer, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA,
    HCURSOR, HICON, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_TIMER, WNDCLASSW, WS_CAPTION,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};

use crate::{logger::background_log, types::VstParameter};

use super::{
    engine::AudioEngine,
    plugin_host::{set_active_vst2_editor_window, VST2_RESIZE_MESSAGE},
    runtime_state::{AudioError, EditorWindow, PluginBackend, VST_EDITOR_IDLE_TIMER_MS},
};

struct WindowData {
    state: std::sync::Weak<Mutex<Option<EditorWindow>>>,
    app_handle: Option<tauri::AppHandle>,
}

const EDITOR_WINDOW_STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0);

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
            #[cfg(target_pointer_width = "64")]
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize)
            };
            #[cfg(target_pointer_width = "32")]
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as i32)
            };
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
                    if let Some(handle) = &(*ptr).app_handle {
                        use tauri::Emitter;
                        let _ = handle.emit("audio:vst-editor-hidden", ());
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            set_active_vst2_editor_window(None);
            let _ = unsafe { KillTimer(Some(hwnd), 1) };
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData };
            if !ptr.is_null() {
                // SAFETY: ptr was allocated with Box::into_raw and is owned by this window.
                let _ = unsafe { Box::from_raw(ptr) };
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            LRESULT(0)
        }
        VST2_RESIZE_MESSAGE => {
            let _ = resize_vst_window_to_client(hwnd, _wparam.0 as i32, lparam.0 as i32);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: lparam is the suggested RECT supplied by Windows for
            // WM_DPICHANGED and remains valid for the duration of this call.
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData };
            if !ptr.is_null() {
                // HIWORD(wParam) is the new Y-axis DPI; Windows currently sends
                // identical X/Y values for a top-level window.
                let dpi = ((_wparam.0 >> 16) & 0xffff).max(96) as f32;
                if let Some(state) = unsafe { (*ptr).state.upgrade() } {
                    if let Some(EditorWindow::Vst3 { gui, .. }) = state.lock().as_mut() {
                        if let Err(error) = gui.set_content_scale_factor(dpi / 96.0) {
                            background_log(
                                "warn",
                                format!("VST3 editor DPI update failed: {error}"),
                            );
                        }
                    }
                }
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
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title_wide.as_ptr()),
            EDITOR_WINDOW_STYLE,
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

        Ok(hwnd)
    }
}

fn resize_vst_window_to_client(hwnd: HWND, width: i32, height: i32) -> Result<(), String> {
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return Err(format!("invalid editor size {width}x{height}"));
    }

    // SAFETY: hwnd is a live top-level window created by create_vst_window. The
    // requested RECT is local and SetWindowPos does not change ownership/z-order.
    unsafe {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let dpi = GetDpiForWindow(hwnd).max(96);
        AdjustWindowRectExForDpi(
            &mut rect,
            EDITOR_WINDOW_STYLE,
            false,
            WINDOW_EX_STYLE(0),
            dpi,
        )
        .map_err(|error| error.to_string())?;
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .map_err(|error| error.to_string())?;
        let mut actual = RECT::default();
        GetClientRect(hwnd, &mut actual).map_err(|error| error.to_string())?;
        let actual_width = actual.right - actual.left;
        let actual_height = actual.bottom - actual.top;
        if actual_width != width || actual_height != height {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                rect.right - rect.left + (width - actual_width),
                rect.bottom - rect.top + (height - actual_height),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

impl AudioEngine {
    pub fn open_vst_ui(&self, app_handle: tauri::AppHandle) -> Result<(), AudioError> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let _ = app_handle;
            return crate::tauri::utils::safe_block_on(self.worker.open_editor())
                .map_err(AudioError::Message);
        }
        let (plugin_arc, editor_window_arc) = {
            let mut guard = self.runtime.lock();
            let Some(runtime) = guard.as_mut() else {
                return Err(AudioError::Message("Audio not started".into()));
            };
            (runtime.plugin.clone(), runtime.editor_window.clone())
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
                        PluginBackend::Vst2 { instance, .. } => {
                            background_log("debug", "VST2 editor: requesting editor object");
                            let editor = instance.get_editor();
                            let Some(mut editor) = editor else {
                                return Err(AudioError::Message(
                                    "Plugin does not expose an editor".into(),
                                ));
                            };

                            let window_data = WindowData {
                                state: weak_editor_window,
                                app_handle: Some(app_handle_clone),
                            };

                            // Some legacy VST2 plugins crash on effEditGetRect before
                            // effEditOpen. Create a hidden neutral parent first, then ask
                            // for the editor size only after the native editor is open.
                            background_log("debug", "VST2 editor: creating hidden parent window");
                            let hwnd = create_vst_window("VST Editor", 800, 600, window_data)
                                .map_err(AudioError::Message)?;

                            let hwnd_ptr = hwnd.0;
                            background_log("debug", "VST2 editor: calling effEditOpen");
                            set_active_vst2_editor_window(Some(hwnd));
                            if editor.open(hwnd_ptr) {
                                editor.idle();
                                let (width, height) = editor.size();
                                match resize_vst_window_to_client(hwnd, width, height) {
                                    Ok(()) => background_log(
                                        "debug",
                                        format!(
                                            "VST2 editor: effEditOpen succeeded; resized client to {width}x{height}"
                                        ),
                                    ),
                                    Err(error) => background_log(
                                        "warn",
                                        format!(
                                            "VST2 editor: post-open size unavailable ({error}); keeping compatibility container 800x600"
                                        ),
                                    ),
                                }
                                unsafe {
                                    let _ = ShowWindow(hwnd, SW_SHOW);
                                    let _ = SetForegroundWindow(hwnd);
                                }
                                editor.idle();
                                let settled_size = editor.size();
                                if settled_size != (width, height) {
                                    if let Err(error) = resize_vst_window_to_client(
                                        hwnd,
                                        settled_size.0,
                                        settled_size.1,
                                    ) {
                                        background_log(
                                            "warn",
                                            format!(
                                                "VST2 editor: settled size correction failed: {error}"
                                            ),
                                        );
                                    }
                                }
                                *editor_window_arc.lock() =
                                    Some(EditorWindow::Vst2 { editor, hwnd });
                                Ok(())
                            } else {
                                set_active_vst2_editor_window(None);
                                unsafe {
                                    let _ = DestroyWindow(hwnd);
                                }
                                Err(AudioError::Message("Failed to open VST editor".into()))
                            }
                        }
                        PluginBackend::Vst3 { instance, .. } => {
                            background_log("debug", "VST3 editor: creating GUI controller");
                            let mut gui = instance.create_gui().map_err(|e| {
                                AudioError::Message(format!("Failed to create VST3 editor: {e}"))
                            })?;
                            background_log("debug", "VST3 editor: querying GUI size");
                            let (width, height) = gui.size().map_err(|e| {
                                AudioError::Message(format!("Failed to get VST3 editor size: {e}"))
                            })?;
                            let client_width = width.round() as i32;
                            let client_height = height.round() as i32;

                            let window_data = WindowData {
                                state: weak_editor_window,
                                app_handle: Some(app_handle_clone),
                            };

                            background_log(
                                "debug",
                                "VST3 editor: creating hidden parent window",
                            );
                            let hwnd = create_vst_window(
                                "VST3 Editor",
                                client_width,
                                client_height,
                                window_data,
                            )
                            .map_err(AudioError::Message)?;
                            if let Err(error) =
                                resize_vst_window_to_client(hwnd, client_width, client_height)
                            {
                                background_log(
                                    "warn",
                                    format!("VST3 editor: could not apply GUI size: {error}"),
                                );
                            }

                            background_log("debug", "VST3 editor: attaching GUI to parent window");
                            if let Err(err) = gui.attach(hwnd.0) {
                                unsafe {
                                    let _ = DestroyWindow(hwnd);
                                }
                                return Err(AudioError::Message(format!(
                                    "Failed to attach VST3 editor: {err}"
                                )));
                            }
                            background_log("debug", "VST3 editor: attach succeeded");

                            *editor_window_arc.lock() = Some(EditorWindow::Vst3 { gui, hwnd });
                            unsafe {
                                let _ = ShowWindow(hwnd, SW_SHOW);
                                let _ = SetForegroundWindow(hwnd);
                            }
                            Ok(())
                        }
                        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
                        PluginBackend::Remote { instance } => {
                            instance.open_editor().map_err(AudioError::Message)
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
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let _ = app_handle;
            return crate::tauri::utils::safe_block_on(self.worker.close_editor())
                .map_err(AudioError::Message);
        }
        let (editor_window_arc, plugin_arc) = {
            let mut guard = self.runtime.lock();
            let Some(runtime) = guard.as_mut() else {
                return Err(AudioError::Message("Audio not started".into()));
            };
            (runtime.editor_window.clone(), runtime.plugin.clone())
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
                }
                #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
                {
                    let mut plugin_guard = plugin_arc.lock();
                    if let PluginBackend::Remote { instance } = &mut *plugin_guard {
                        let _ = instance.close_editor();
                    }
                }
                #[cfg(not(all(target_os = "windows", target_pointer_width = "64")))]
                let _ = plugin_arc;
            })
            .map_err(|_| {
                AudioError::Message("Failed to schedule VST close on main thread".into())
            })?;

        Ok(())
    }

    pub fn list_vst_parameters(&self) -> Result<Vec<VstParameter>, AudioError> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return crate::tauri::utils::safe_block_on(self.worker.list_parameters())
                .map_err(AudioError::Message);
        }
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            return Err(AudioError::Message("Audio not started".into()));
        };

        let parameters = runtime.parameter_cache.lock().clone();
        Ok(parameters)
    }

    pub fn set_vst_parameter(&self, index: usize, value: f32) -> Result<(), AudioError> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return crate::tauri::utils::safe_block_on(self.worker.set_parameter(index, value))
                .map_err(AudioError::Message);
        }
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            return Err(AudioError::Message("Audio not started".into()));
        };
        let normalized = value.clamp(0.0, 1.0);
        let metadata = runtime
            .parameter_cache
            .lock()
            .get(index)
            .cloned()
            .ok_or_else(|| AudioError::Message("VST parameter index out of range".into()))?;
        runtime
            .parameter_tx
            .try_send(super::runtime_state::ParameterCommand {
                index,
                id: metadata.id,
                flags: metadata.flags,
                step_count: metadata.step_count,
                value: normalized,
            })
            .map_err(|_| AudioError::Message("VST parameter queue is full".into()))?;
        if let Some(parameter) = runtime.parameter_cache.lock().get_mut(index) {
            parameter.value = normalized;
        }
        Ok(())
    }
}
