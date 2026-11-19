use anyhow::{Context, Result};
use async_channel::{Receiver, bounded};
use gpui::Window;
use pulseboard::diagnostics::{SessionLog, exception_line};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    backtrace::Backtrace,
    ffi::{OsStr, c_void},
    io::ErrorKind,
    os::windows::{ffi::OsStrExt, io::IntoRawHandle},
    path::PathBuf,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicPtr, Ordering},
    },
};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND},
    Graphics::Dwm::DwmSetWindowAttribute,
    Storage::FileSystem::WriteFile,
    System::{
        Diagnostics::Debug::{
            EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
        },
        Threading::CreateMutexW,
    },
    UI::{
        Shell::SetCurrentProcessExplicitAppUserModelID,
        WindowsAndMessaging::{
            FindWindowW, IsWindowVisible, MB_ICONERROR, MessageBoxW, SW_HIDE, SW_RESTORE, SW_SHOW,
            SetForegroundWindow, ShowWindowAsync,
        },
    },
};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

const APP_ID: &str = "Pulseboard.Desktop";
const MUTEX_NAME: &str = "Local\\Pulseboard.Desktop";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

static CRASH_LOG: OnceLock<SessionLog> = OnceLock::new();
static EXCEPTION_LOG_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

pub fn install_crash_log() {
    let Ok(path) = pulseboard::config::path() else {
        return;
    };
    let Some(directory) = path.parent() else {
        return;
    };
    let Ok(log) = SessionLog::start(directory) else {
        return;
    };
    if CRASH_LOG.set(log).is_err() {
        return;
    }
    if let Some(log) = CRASH_LOG.get()
        && let Ok(file) = log.open()
    {
        // Keep the append-only handle open until process exit so the exception filter
        // never needs to open a path or allocate memory on a faulting thread.
        EXCEPTION_LOG_HANDLE.store(file.into_raw_handle(), Ordering::Release);
        unsafe { SetUnhandledExceptionFilter(Some(log_native_exception)) };
    }

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        if let Some(log) = CRASH_LOG.get() {
            // The panic payload might contain endpoint details or credentials.
            let _ = log.record(format_args!(
                "Rust panic at {:?}\n{}",
                panic.location(),
                Backtrace::force_capture()
            ));
        }
        previous(panic);
    }));
}

unsafe extern "system" fn log_native_exception(exception: *const EXCEPTION_POINTERS) -> i32 {
    let handle = EXCEPTION_LOG_HANDLE.load(Ordering::Acquire);
    if !handle.is_null()
        && let Some(exception) = unsafe { exception.as_ref() }
        && let Some(record) = unsafe { exception.ExceptionRecord.as_ref() }
    {
        let line = exception_line(record.ExceptionCode as u32);
        let mut written = 0;
        unsafe {
            WriteFile(
                handle,
                line.as_ptr(),
                line.len() as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
    }
    EXCEPTION_CONTINUE_SEARCH
}

pub fn diagnostic_event(event: &'static str) {
    if let Some(log) = CRASH_LOG.get() {
        let _ = log.record(format_args!("{event}"));
    }
}

pub fn finish_crash_log() {
    if let Some(log) = CRASH_LOG.get() {
        let _ = log.finish();
    }
}

pub fn crash_log_path() -> Option<PathBuf> {
    CRASH_LOG.get().map(SessionLog::path)
}

pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // The instance object owns this handle for the lifetime of the app.
        unsafe { CloseHandle(self.0) };
    }
}

pub fn acquire_instance() -> std::io::Result<Option<Instance>> {
    let mutex_name = wide(OsStr::new(MUTEX_NAME));
    let handle = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let instance = Instance(handle);
    if exists {
        let title = wide(OsStr::new("Pulseboard"));
        let window = unsafe { FindWindowW(ptr::null(), title.as_ptr()) };
        if !window.is_null() {
            show(window);
        }
        return Ok(None);
    }
    let app_id = wide(OsStr::new(APP_ID));
    unsafe { SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr()) };
    Ok(Some(instance))
}

pub fn show_error(message: &str) {
    let message = wide(OsStr::new(message));
    let title = wide(OsStr::new("Pulseboard"));
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR,
        )
    };
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

fn hwnd(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

pub fn decorate(window: &Window, dark: bool) {
    if let Some(hwnd) = hwnd(window) {
        let dark_mode = u32::from(dark);
        unsafe { DwmSetWindowAttribute(hwnd, 20, (&raw const dark_mode).cast(), 4) };
    }
}

pub fn hide(window: &Window) -> bool {
    let Some(hwnd) = hwnd(window) else {
        return false;
    };
    unsafe { ShowWindowAsync(hwnd, SW_HIDE) != 0 }
}

pub fn restore(window: &Window) {
    if let Some(hwnd) = hwnd(window)
        && unsafe { IsWindowVisible(hwnd) } == 0
    {
        unsafe { ShowWindowAsync(hwnd, SW_SHOW) };
    }
    window.activate_window();
}

fn show(hwnd: HWND) {
    unsafe {
        ShowWindowAsync(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}

pub enum TrayAction {
    Open,
    Run,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    pub events: Receiver<TrayAction>,
}

impl Tray {
    pub fn new() -> Result<Self> {
        let menu = Menu::new();
        let open = MenuItem::with_id("open", "Pulseboard öffnen", true, None);
        let run = MenuItem::with_id("run", "Jetzt prüfen", true, None);
        let quit = MenuItem::with_id("quit", "Beenden", true, None);
        menu.append_items(&[&open, &run, &quit])?;

        let image = image::load_from_memory_with_format(
            include_bytes!("../installer/pulseboard.ico"),
            image::ImageFormat::Ico,
        )?
        .into_rgba8();
        let (width, height) = image.dimensions();
        let icon = TrayIconBuilder::new()
            .with_tooltip("Pulseboard")
            .with_menu_on_left_click(false)
            .with_menu(Box::new(menu))
            .with_icon(Icon::from_rgba(image.into_raw(), width, height)?)
            .build()?;
        let (send, events) = bounded(16);
        let click_send = send.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = click_send.try_send(TrayAction::Open);
            }
        }));
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                "open" => TrayAction::Open,
                "run" => TrayAction::Run,
                "quit" => TrayAction::Quit,
                _ => return,
            };
            let _ = send.try_send(action);
        }));
        Ok(Self { icon, events })
    }

    pub fn update_tooltip(&self, healthy: usize, failed: usize, total: usize) {
        let tooltip = if total == 0 {
            "Pulseboard · Keine Endpunkte".into()
        } else {
            format!("Pulseboard · {healthy}/{total} erreichbar · {failed} Fehler")
        };
        let _ = self.icon.set_tooltip(Some(tooltip));
    }
}

pub fn notify(title: &str, message: &str) -> Result<()> {
    winrt_notification::Toast::new(APP_ID)
        .title(title)
        .text1(message)
        .show()
        .map_err(|_| anyhow::anyhow!("Windows-Benachrichtigung konnte nicht zugestellt werden"))
}

pub fn autostart_enabled() -> Result<bool> {
    let user = RegKey::predef(HKEY_CURRENT_USER);
    match user
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>("Pulseboard"))
    {
        Ok(value) => Ok(value == autostart_command()?),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("Autostart konnte nicht gelesen werden"),
    }
}

pub fn set_autostart(enabled: bool) -> Result<()> {
    let user = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = user.create_subkey(RUN_KEY)?;
    if enabled {
        key.set_value("Pulseboard", &autostart_command()?)?;
    } else if let Err(error) = key.delete_value("Pulseboard")
        && error.kind() != ErrorKind::NotFound
    {
        return Err(error.into());
    }
    Ok(())
}

fn autostart_command() -> Result<String> {
    let path = std::env::current_exe()?;
    let path = path
        .to_str()
        .context("Programmpfad enthält ungültige Zeichen")?;
    Ok(format!("\"{path}\" --background"))
}
