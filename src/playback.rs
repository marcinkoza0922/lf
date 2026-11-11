//! libmpv is loaded lazily; all native handles and rendering stay on one worker.
use eframe::egui;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

pub enum Command {
    Pause(bool),
    Seek(f64),
    Volume(f64),
}
#[derive(Default)]
pub struct Snapshot {
    pub frame: Option<egui::ColorImage>,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub ended: bool,
    pub loaded: bool,
    pub error: Option<String>,
}
pub struct Playback {
    pub state: Arc<Mutex<Snapshot>>,
    sender: Option<mpsc::Sender<Command>>,
}
impl Playback {
    pub fn start(target: String, video: bool, ctx: egui::Context) -> Self {
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let (sender, receiver) = mpsc::channel();
        let output = state.clone();
        std::thread::spawn(move || {
            if let Err(error) = run(&target, video, &receiver, &output, &ctx) {
                output.lock().unwrap().error = Some(error);
                ctx.request_repaint();
            }
        });
        Self {
            state,
            sender: Some(sender),
        }
    }
    pub fn send(&self, command: Command) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(command);
        }
    }
}
// Dropping the sender disconnects the worker, which frees the renderer before
// terminating mpv. Never join a possibly slow decoder on the UI thread.
impl Drop for Playback {
    fn drop(&mut self) {
        self.sender.take();
    }
}

#[repr(C)]
struct Param {
    kind: c_int,
    data: *mut c_void,
}
impl Param {
    fn new(kind: c_int, data: *mut c_void) -> Self {
        Self { kind, data }
    }
    fn end() -> Self {
        Self::new(0, ptr::null_mut())
    }
}
#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct Property {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

// These signatures and repr(C) layouts follow mpv's public client.h/render.h.
// The library outlives every function pointer and native handle.
struct Api {
    _library: libloading::Library,
    create: unsafe extern "C" fn() -> *mut c_void,
    initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    option: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    observe: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    command: unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> c_int,
    event: unsafe extern "C" fn(*mut c_void, f64) -> *const Event,
    error: unsafe extern "C" fn(c_int) -> *const c_char,
    terminate: unsafe extern "C" fn(*mut c_void),
    render_create: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut Param) -> c_int,
    render_update: unsafe extern "C" fn(*mut c_void) -> u64,
    render: unsafe extern "C" fn(*mut c_void, *mut Param) -> c_int,
    render_free: unsafe extern "C" fn(*mut c_void),
}
impl Api {
    fn load() -> Result<&'static Self, String> {
        // Keep libmpv resident: dependencies can retain thread-local destructors
        // even after a player has terminated. Unloading could leave stale code pointers.
        static API: std::sync::OnceLock<Result<Api, String>> = std::sync::OnceLock::new();
        API.get_or_init(Self::open).as_ref().map_err(Clone::clone)
    }
    fn open() -> Result<Self, String> {
        // SAFETY: load only the system libmpv SONAME; each symbol's signature
        // matches the stable public C ABI. No document-controlled library paths.
        unsafe {
            let library = libloading::Library::new("libmpv.so.2").map_err(|_| {
                "Inline playback requires libmpv.so.2 (install the mpv-libs / libmpv2 package)."
                    .to_string()
            })?;
            macro_rules! symbol {
                ($name:literal) => {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| e.to_string())?
                };
            }
            Ok(Self {
                create: symbol!("mpv_create"),
                initialize: symbol!("mpv_initialize"),
                option: symbol!("mpv_set_option_string"),
                observe: symbol!("mpv_observe_property"),
                command: symbol!("mpv_command_async"),
                event: symbol!("mpv_wait_event"),
                error: symbol!("mpv_error_string"),
                terminate: symbol!("mpv_terminate_destroy"),
                render_create: symbol!("mpv_render_context_create"),
                render_update: symbol!("mpv_render_context_update"),
                render: symbol!("mpv_render_context_render"),
                render_free: symbol!("mpv_render_context_free"),
                _library: library,
            })
        }
    }
    fn check(&self, code: c_int) -> Result<(), String> {
        if code >= 0 {
            return Ok(());
        }
        // SAFETY: mpv_error_string returns a static NUL-terminated string.
        Err(unsafe { CStr::from_ptr((self.error)(code)) }
            .to_string_lossy()
            .into_owned())
    }
}
struct Native {
    api: &'static Api,
    handle: *mut c_void,
    renderer: *mut c_void,
}
impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: owned handles are used only on this worker. The renderer must
        // be destroyed before its mpv core, including on initialization errors.
        unsafe {
            if !self.renderer.is_null() {
                (self.api.render_free)(self.renderer);
            }
            if !self.handle.is_null() {
                (self.api.terminate)(self.handle);
            }
        }
    }
}
impl Native {
    fn new(video: bool) -> Result<Self, String> {
        Self::with_audio(video, false)
    }
    fn with_audio(video: bool, silent: bool) -> Result<Self, String> {
        let api = Api::load()?;
        // SAFETY: library is loaded and the returned handle is uniquely owned.
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("Could not create media player".into());
        }
        let mut native = Self {
            api,
            handle,
            renderer: ptr::null_mut(),
        };
        // SAFETY: strings are NUL-terminated and mpv copies option/observation
        // arguments. All synchronous setup precedes render-context creation.
        unsafe {
            for (name, value) in [
                (c"ao", if silent { c"null" } else { c"" }),
                (c"config", c"no"),
                (c"terminal", c"no"),
                (c"ytdl", c"no"),
                (c"load-scripts", c"no"),
                (c"input-default-bindings", c"no"),
                (c"vo", if video { c"libmpv" } else { c"null" }),
                (c"vid", if video { c"auto" } else { c"no" }),
                (c"idle", c"yes"),
                (c"keep-open", c"yes"),
            ] {
                native
                    .api
                    .check((native.api.option)(handle, name.as_ptr(), value.as_ptr()))
                    .map_err(|error| format!("{}: {error}", name.to_string_lossy()))?;
            }
            native.api.check((native.api.initialize)(handle))?;
            for (id, name, format) in [
                (1, c"time-pos", 5),
                (2, c"duration", 5),
                (3, c"pause", 3),
                (4, c"eof-reached", 3),
            ] {
                native
                    .api
                    .check((native.api.observe)(handle, id, name.as_ptr(), format))?;
            }
            if video {
                let mut advanced: c_int = 1;
                let mut params = [
                    Param::new(1, c"sw".as_ptr().cast_mut().cast()),
                    Param::new(10, (&mut advanced as *mut c_int).cast()),
                    Param::end(),
                ];
                native.api.check((native.api.render_create)(
                    &mut native.renderer,
                    handle,
                    params.as_mut_ptr(),
                ))?;
            }
        }
        Ok(native)
    }
    fn command(&self, args: &[&str]) -> Result<(), String> {
        let strings: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let mut pointers: Vec<_> = strings.iter().map(|s| s.as_ptr()).collect();
        pointers.push(ptr::null());
        // SAFETY: mpv copies all arguments before returning from the async call.
        self.api
            .check(unsafe { (self.api.command)(self.handle, 0, pointers.as_ptr()) })
    }
    fn events(&self, snapshot: &mut Snapshot) -> Result<bool, String> {
        let mut changed = false;
        // SAFETY: event data is owned by mpv and read before the next wait_event.
        // Formats are checked before dereferencing property payloads.
        unsafe {
            loop {
                let event = &*(self.api.event)(self.handle, 0.0);
                if event.id == 0 {
                    break;
                }
                changed = true;
                self.api.check(event.error)?;
                if event.id == 8 {
                    snapshot.loaded = true;
                }
                if event.id == 7 && !event.data.is_null() {
                    let end = &*event.data.cast::<EndFile>();
                    self.api.check(end.error)?;
                    snapshot.ended = true;
                }
                if event.id == 22 && !event.data.is_null() {
                    let property = &*event.data.cast::<Property>();
                    if property.data.is_null() {
                        continue;
                    }
                    match (event.userdata, property.format) {
                        (1, 5) => snapshot.position = *property.data.cast::<f64>(),
                        (2, 5) => snapshot.duration = *property.data.cast::<f64>(),
                        (3, 3) => snapshot.paused = *property.data.cast::<c_int>() != 0,
                        (4, 3) => snapshot.ended = *property.data.cast::<c_int>() != 0,
                        _ => {}
                    }
                }
            }
        }
        Ok(changed)
    }
    fn frame(&self, pixels: &mut [u32]) -> Result<bool, String> {
        if self.renderer.is_null() {
            return Ok(false);
        }
        // SAFETY: renderer belongs to this thread. rgb0 requires 4-byte pixel
        // alignment; Vec<u32> supplies it. Size/stride fit the allocated buffer.
        unsafe {
            if (self.api.render_update)(self.renderer) & 1 == 0 {
                return Ok(false);
            }
            let mut size = [640i32, 360i32];
            let mut stride = 640usize * 4;
            let mut params = [
                Param::new(17, size.as_mut_ptr().cast()),
                Param::new(18, c"rgb0".as_ptr().cast_mut().cast()),
                Param::new(19, (&mut stride as *mut usize).cast()),
                Param::new(20, pixels.as_mut_ptr().cast()),
                Param::end(),
            ];
            self.api
                .check((self.api.render)(self.renderer, params.as_mut_ptr()))?;
        }
        Ok(true)
    }
}
fn run(
    target: &str,
    video: bool,
    receiver: &mpsc::Receiver<Command>,
    output: &Arc<Mutex<Snapshot>>,
    ctx: &egui::Context,
) -> Result<(), String> {
    let native = Native::new(video)?;
    native.command(&["loadfile", target])?;
    let mut pixels = vec![0u32; 640 * 360];
    loop {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(Command::Pause(paused)) => {
                native.command(&["set", "pause", if paused { "yes" } else { "no" }])?
            }
            Ok(Command::Seek(position)) => {
                native.command(&["seek", &position.to_string(), "absolute+exact"])?;
            }
            Ok(Command::Volume(volume)) => {
                native.command(&["set", "volume", &volume.to_string()])?
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // Rendering can block for video timing. Never hold the UI's state lock.
        let frame = if native.frame(&mut pixels)? {
            let colors = pixels
                .iter()
                .map(|pixel| {
                    let [r, g, b, _] = pixel.to_ne_bytes();
                    egui::Color32::from_rgb(r, g, b)
                })
                .collect();
            Some(egui::ColorImage::new([640, 360], colors))
        } else {
            None
        };
        let mut state = output.lock().unwrap();
        let changed = native.events(&mut state)?;
        let rendered = frame.is_some();
        if rendered {
            state.frame = frame;
        }
        drop(state);
        if changed || rendered {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::Instant};
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn fixture(video: bool) -> Fixture {
        let path = std::env::temp_dir().join(format!(
            "md-reader-media {} {}.{}",
            std::process::id(),
            video,
            if video { "mkv" } else { "wav" }
        ));
        let mut command = std::process::Command::new("ffmpeg");
        command.args(["-v", "error", "-y", "-f", "lavfi"]);
        if video {
            command.args([
                "-i",
                "testsrc2=size=160x90:rate=15",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-c:v",
                "mpeg4",
                "-c:a",
                "pcm_s16le",
            ]);
        } else {
            command.args([
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-c:a",
                "pcm_s16le",
            ]);
        }
        assert!(
            command
                .args(["-t", "2"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        Fixture(path)
    }
    fn wait(native: &Native, state: &mut Snapshot, predicate: impl Fn(&Snapshot, bool) -> bool) {
        let start = Instant::now();
        let mut pixels = vec![0u32; 640 * 360];
        loop {
            let frame = native.frame(&mut pixels).unwrap();
            native.events(state).unwrap();
            if predicate(state, frame) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "media timed out: loaded={}, pos={}, duration={}",
                state.loaded,
                state.position,
                state.duration
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "Requires libmpv.so.2 and ffmpeg; see DESIGN.md"]
    fn audio_decodes_pauses_seeks_and_reaches_eof() {
        let fixture = fixture(false);
        let native = Native::with_audio(false, true).unwrap();
        let target = url::Url::from_file_path(&fixture.0).unwrap();
        native.command(&["loadfile", target.as_str()]).unwrap();
        let mut state = Snapshot::default();
        wait(&native, &mut state, |s, _| {
            s.loaded && s.position > 0.05 && s.duration > 1.9
        });
        native.command(&["set", "pause", "yes"]).unwrap();
        wait(&native, &mut state, |s, _| s.paused);
        native.command(&["seek", "1.5", "absolute+exact"]).unwrap();
        wait(&native, &mut state, |s, _| s.position >= 1.4);
        native.command(&["set", "volume", "35"]).unwrap();
        native.command(&["set", "pause", "no"]).unwrap();
        wait(&native, &mut state, |s, _| s.ended);
        native.command(&["seek", "0", "absolute+exact"]).unwrap();
        native.command(&["set", "pause", "no"]).unwrap();
        wait(&native, &mut state, |s, _| !s.ended && s.position < 0.5);
    }
    #[test]
    #[ignore = "Requires libmpv.so.2 and ffmpeg; see DESIGN.md"]
    fn video_renders_pixels_and_reports_missing_files() {
        let fixture = fixture(true);
        let native = Native::with_audio(true, true).unwrap();
        native
            .command(&["loadfile", fixture.0.to_str().unwrap()])
            .unwrap();
        let mut state = Snapshot::default();
        wait(&native, &mut state, |s, frame| {
            s.loaded && frame && s.position > 0.05
        });
        let mut pixels = vec![0u32; 640 * 360];
        let start = Instant::now();
        while !native.frame(&mut pixels).unwrap() {
            native.events(&mut state).unwrap();
            assert!(start.elapsed() < Duration::from_secs(8));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            pixels
                .iter()
                .any(|pixel| pixel.to_ne_bytes()[..3] != [0, 0, 0])
        );
        native
            .command(&["loadfile", "/nonexistent/md-reader-missing.mp4"])
            .unwrap();
        let start = Instant::now();
        loop {
            let _ = native.frame(&mut pixels);
            if native.events(&mut state).is_err() {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(8));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
