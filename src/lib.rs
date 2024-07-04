//! A logger for use with Windows debuggers.
//!
//! This crate integrates with the ubiquitous [`log`] crate.
//!
//! Windows allows applications to output a string directly to debuggers. This is
//! very useful in situations where other forms of logging are not available.
//! For example, stderr is not available for GUI apps.
//!
//! Windows provides the `OutputDebugString` entry point, which allows apps to
//! print a debug string. Internally, `OutputDebugString` is implemented by
//! raising an SEH exception, which the debugger catches and handles.
//!
//! Raising an exception has a significant cost, when run under a debugger,
//! because the debugger halts all threads in the target process. So you should
//! avoid using this logger for high rates of output, because doing so will
//! slow down your app.
//!
//! Like many Windows entry points, `OutputDebugString` is actually two entry
//! points: `OutputDebugStringA` (multi-byte encodings) and
//! `OutputDebugStringW` (UTF-16). In most cases, the `*A` version is implemented
//! using a "thunk" which converts its arguments to UTF-16 and then calls the `*W`
//! version. However, `OutputDebugStringA` is one of the few entry points where
//! the opposite is true.
//!
//! This crate can be compiled and used on non-Windows platforms, but it does
//! nothing. This is intended to minimize the impact on code that takes a
//! dependency on this crate.
//!
//! # Example
//!
//! ```rust
//! use log::{debug, info};
//!
//! fn do_cool_stuff() {
//!    info!("Hello, world!");
//!    debug!("Hello, world, in detail!");
//! }
//!
//! fn main() {
//!     log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER).unwrap();
//!     log::set_max_level(log::LevelFilter::Debug);
//!
//!     do_cool_stuff();
//! }
//! ```

use log::{Level, LevelFilter, Metadata, Record};

/// This implements `log::Log`, and so can be used as a logging provider.
/// It forwards log messages to the Windows `OutputDebugString` API.
pub struct DebuggerLogger {
    /// Force logging without debugger attached. Useful with DbgView
    force_log_without_debugger: std::sync::atomic::AtomicBool,
}

/// This is a static instance of `DebuggerLogger`. Since `DebuggerLogger`
/// contains no state, this can be directly registered using `log::set_logger`.
///
/// Example:
///
/// ```
/// // During initialization:
/// log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER).unwrap();
/// log::set_max_level(log::LevelFilter::Debug);
///
/// // Throughout your code:
/// use log::{info, debug};
///
/// info!("Hello, world!");
/// debug!("Hello, world, in detail!");
/// ```
pub static DEBUGGER_LOGGER: DebuggerLogger = DebuggerLogger {
    force_log_without_debugger: std::sync::atomic::AtomicBool::new(false),
};

impl DebuggerLogger {
    /// Set logging wihout debugger attached flag.
    pub fn set_force_log_without_debugger(&self, v: bool) {
        self.force_log_without_debugger
            .store(v, std::sync::atomic::Ordering::Relaxed)
    }
}

impl log::Log for DebuggerLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        self.force_log_without_debugger
            .load(std::sync::atomic::Ordering::Relaxed)
            || is_debugger_present()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let s = format!(
                "{}({}): {} - {}\r\n",
                record.file().unwrap_or("<unknown>"),
                record.line().unwrap_or(0),
                record.level(),
                record.args()
            );
            output_debug_string(&s);
        }
    }

    fn flush(&self) {}
}

/// Calls the `OutputDebugString` API to log a string.
///
/// On non-Windows platforms, this function does nothing.
///
/// See [`OutputDebugStringW`](https://docs.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-outputdebugstringw).
pub fn output_debug_string(s: &str) {
    #[cfg(windows)]
    {
        let len = s.encode_utf16().count() + 1;
        let mut s_utf16: Vec<u16> = Vec::with_capacity(len);
        s_utf16.extend(s.encode_utf16());
        s_utf16.push(0);
        unsafe {
            OutputDebugStringW(&s_utf16[0]);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = s;
    }
}

#[cfg(windows)]
extern "stdcall" {
    fn OutputDebugStringW(chars: *const u16);
    fn IsDebuggerPresent() -> i32;
}

/// Checks whether a debugger is attached to the current process.
///
/// On non-Windows platforms, this function always returns `false`.
///
/// See [`IsDebuggerPresent`](https://docs.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-isdebuggerpresent).
pub fn is_debugger_present() -> bool {
    #[cfg(windows)]
    {
        unsafe { IsDebuggerPresent() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Sets the `DebuggerLogger` as the currently-active logger.
///
/// If an error occurs when registering `DebuggerLogger` as the current logger,
/// this function will output a warning and will return normally. It will not panic.
/// This behavior was chosen because `DebuggerLogger` is intended for use in debugging.
/// Panicking would disrupt debugging and introduce new failure modes. It would also
/// create problems for mixed-mode debugging, where Rust code is linked with C/C++ code.
pub fn init() {
    match log::set_logger(&DEBUGGER_LOGGER) {
        Ok(()) => {}
        Err(_) => {
            // There's really nothing we can do about it.
            output_debug_string(
                "Warning: Failed to register DebuggerLogger as the current Rust logger.\r\n",
            );
        }
    }
}

macro_rules! define_init_at_level {
    ($func:ident, $level:ident) => {
        /// This can be called from C/C++ code to register the debug logger.
        ///
        /// For Windows DLLs that have statically linked an instance of `win_dbg_logger` into them,
        /// `DllMain` should call `win_dbg_logger_init_<level>()` from the `DLL_PROCESS_ATTACH` handler.
        /// For example:
        ///
        /// ```ignore
        /// // Calls into Rust code.
        /// extern "C" void __cdecl rust_win_dbg_logger_init_debug();
        ///
        /// BOOL WINAPI DllMain(HINSTANCE hInstance, DWORD reason, LPVOID reserved) {
        ///     switch (reason) {
        ///         case DLL_PROCESS_ATTACH:
        ///             rust_win_dbg_logger_init_debug();
        ///             // ...
        ///     }
        ///     // ...
        /// }
        /// ```
        ///
        /// For Windows executables that have statically linked an instance of `win_dbg_logger` into
        /// them, call `win_dbg_logger_init_<level>()` during app startup.
        #[no_mangle]
        pub extern "C" fn $func() {
            init();
            log::set_max_level(LevelFilter::$level);
        }
    };
}

define_init_at_level!(rust_win_dbg_logger_init_info, Info);
define_init_at_level!(rust_win_dbg_logger_init_trace, Trace);
define_init_at_level!(rust_win_dbg_logger_init_debug, Debug);
define_init_at_level!(rust_win_dbg_logger_init_warn, Warn);
define_init_at_level!(rust_win_dbg_logger_init_error, Error);
