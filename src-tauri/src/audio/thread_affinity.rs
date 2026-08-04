use std::sync::OnceLock;

static MAIN_UI_THREAD_ID: OnceLock<std::thread::ThreadId> = OnceLock::new();

pub(crate) fn register_main_ui_thread() {
    let _ = MAIN_UI_THREAD_ID.set(std::thread::current().id());
}

pub(super) fn is_main_ui_thread() -> bool {
    MAIN_UI_THREAD_ID
        .get()
        .is_some_and(|thread_id| *thread_id == std::thread::current().id())
}
