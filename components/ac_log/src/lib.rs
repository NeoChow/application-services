use crossbeam_channel::Sender;
use std::{
    ffi::CString,
    os::raw::c_char,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

#[derive(Clone, Copy)]
#[repr(i32)]
pub enum LogLevel {
    // Android logger levels
    VERBOSE = 2,
    DEBUG = 3,
    INFO = 4,
    WARN = 5,
    ERROR = 6,
}

impl From<log::Level> for LogLevel {
    fn from(l: log::Level) -> Self {
        match l {
            log::Level::Trace => LogLevel::VERBOSE,
            log::Level::Debug => LogLevel::DEBUG,
            log::Level::Info => LogLevel::INFO,
            log::Level::Warn => LogLevel::WARN,
            log::Level::Error => LogLevel::ERROR,
        }
    }
}

// TODO: use serde to send this to the other thread as bincode or something,
// rather than allocating all these strings for every message.
struct LogRecord {
    level: LogLevel,
    tag: Option<CString>,
    message: CString,
}

fn string_to_cstring_lossy(s: String) -> CString {
    let mut bytes = s.into_bytes();
    for byte in bytes.iter_mut() {
        if *byte == 0 {
            *byte = b'?';
        }
    }
    CString::new(bytes).expect("Bug in string_to_cstring_lossy!")
}

impl<'a, 'b> From<&'b log::Record<'a>> for LogRecord {
    // XXX important! Don't log in this function!
    fn from(r: &'b log::Record<'a>) -> Self {
        let message = match (r.line(), r.file()) {
            (Some(line), Some(file)) => format!("{}@{}: {}", file, line, r.args()),
            (None, Some(file)) => format!("{}: {}", file, r.args()),
            // The (Some(line), None) case is pointless
            (_, None) => format!("{}", r.args()),
        };

        Self {
            level: r.level().into(),
            tag: r
                .module_path()
                .and_then(|mp| CString::new(mp.to_owned()).ok()),
            message: string_to_cstring_lossy(message),
        }
    }
}

/// Type of the log callback provided to us by java.
/// Takes the following arguments:
///
/// - Log level (an i32).
/// - Tag: a (nullable) nul terminated c string. Caller must not free this string!
/// - Message: a (non-nullable) nul terminated c string. Caller must not free this string!
pub type LogCallback = extern "C" fn(LogLevel, *const c_char, *const c_char);

pub struct LogAdapterState {
    // Thread handle for the BG thread. We can't drop this without problems so weu32
    // prefix with _ to shut rust up about it being unused.
    handle: Option<std::thread::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
    done_sender: Sender<()>,
}

pub struct LogSink {
    stopped: Arc<AtomicBool>,
    sender: Sender<LogRecord>,
}

impl log::Log for LogSink {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        // Really this could just be Acquire but whatever
        !self.stopped.load(Ordering::SeqCst)
    }

    fn flush(&self) {}
    fn log(&self, record: &log::Record) {
        // Important: we check stopped before writing, which means
        // it must be set before
        if self.stopped.load(Ordering::SeqCst) {
            // Note: `enabled` is not automatically called.
            return;
        }
        // In practice this should never fail, we always set `stopped` before
        // closing the channel. That said, in the future it wouldn't be
        // unreasonable to swallow this error.
        self.sender.send(record.into()).unwrap();
    }
}

impl LogAdapterState {
    pub fn init(callback: LogCallback) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let (record_sender, record_recv) = crossbeam_channel::unbounded();
        // We use a channel to notify the `drain` thread that we changed done,
        // so that we can close it in a timely fashion.
        let (done_sender, done_recv) = crossbeam_channel::bounded(1);
        let handle = {
            let stopped = stopped.clone();
            thread::spawn(move || {
                loop {
                    // XXX explain why we need this mess instead of just e.g. waiting for Err
                    crossbeam_channel::select! {
                        recv(record_recv) -> record => {
                            if stopped.load(Ordering::SeqCst) {
                                return;
                            }
                            if let Ok(LogRecord { level, tag, message }) = record {
                                let tag_ptr = tag.as_ref()
                                    .map(|s| s.as_ptr())
                                    .unwrap_or_else(std::ptr::null);
                                let msg_ptr = message.as_ptr();
                                callback(level, tag_ptr, msg_ptr);
                            } else {
                                // Channel closed.
                                stopped.store(true, Ordering::SeqCst);
                                return;
                            }
                        },
                        recv(done_recv) -> _ => {
                            return;
                        }
                    };

                    // Could be Acquire
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                }
            })
        };
        let sink = LogSink {
            sender: record_sender,
            stopped: stopped.clone(),
        };

        log::set_max_level(log::LevelFilter::max());
        log::set_boxed_logger(Box::new(sink)).unwrap();
        log::info!("ac_log adapter initialized!");
        Self {
            handle: Some(handle),
            stopped,
            done_sender,
        }
    }

    pub fn stop(&mut self) {}
}

impl Drop for LogAdapterState {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.done_sender.send(()).unwrap();
        // TODO: can we safely return from this (I suspect the answer is no, and
        // we have to panic and abort higher up...)
        if let Some(h) = self.handle.take() {
            h.join().unwrap();
        }
    }
}

ffi_support::implement_into_ffi_by_pointer!(LogAdapterState);
ffi_support::define_string_destructor!(ac_log_adapter_destroy_string);

#[no_mangle]
pub extern "C" fn ac_log_adapter_create(
    callback: LogCallback,
    out_err: &mut ffi_support::ExternError,
) -> *mut LogAdapterState {
    ffi_support::call_with_output(out_err, || LogAdapterState::init(callback))
}

// Can't use define_box_destructor because this can panic. TODO: Maybe we should
// keep this around globally (as lazy_static or something) and basically just
// turn it on/off in create/destroy... Might be more reliable?
#[no_mangle]
pub unsafe extern "C" fn ac_log_adapter_destroy(to_destroy: *mut LogAdapterState) {
    ffi_support::abort_on_panic::call_with_output(|| {
        drop(Box::from_raw(to_destroy));
    })
}

// Used just to allow tests to produce logs.
#[no_mangle]
pub unsafe extern "C" fn ac_log_adapter_test__log_msg(msg: *const c_char) {
    ffi_support::abort_on_panic::call_with_output(|| {
        log::info!("testing: {}", ffi_support::rust_str_from_c(msg));
    });
}
