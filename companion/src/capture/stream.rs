//! The strip read from one ScreenCaptureKit stream on the game's window, instead of a new
//! screencapture process for every look. Each one-shot capture costs macOS's systemstatusd
//! about a third of a second of CPU, and more as a session goes on, until a look takes half
//! a second and the strip is read twice a second at best. A stream that stays open costs it
//! next to nothing, and a look is a copy out of memory.
//!
//! The stream covers the window's top-left corner (all the strip search looks at) and keeps
//! the newest frame. macOS sends a frame only when the content changes, so an old frame is
//! still what is on screen. It follows one window: a new window, size or scale opens a new
//! stream, and so does macOS stopping it (sleep, a display change). Until a stream runs, and
//! when one can't, the looks fall back to screencapture, less often while the strip hides.

use super::mac::{Grab, GrabError, Image, Rect, WindowFrame};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Frames at most this often: twice the looks, so a look never waits long for one.
const FRAMES_PER_SECOND: i32 = 8;
/// Frames macOS keeps ready; few, as only the newest counts.
const QUEUE_DEPTH: isize = 3;
/// A new stream sends its first frame at once; one that hasn't within this has failed.
const FIRST_FRAME_WITHIN: Duration = Duration::from_secs(3);
/// How long to wait for macOS to list the windows, or to start a stream.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);
/// Waits before starting a stream again after one failed or stopped: this, doubling with each
/// failure in a row, up to `LAST_RETRY`.
const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(300);
/// A stream that has run this long works: the next failure waits `FIRST_RETRY` again.
const PROVEN_AFTER: Duration = Duration::from_secs(60);

/// Looks at the strip: four times a second from a stream, or from screencapture while the
/// strip was seen in the last `STRIP_RECENT`. Otherwise screencapture looks less often, as
/// the strip only shows while the game has something to send.
const TICK: Duration = Duration::from_millis(250);
const IDLE_TICK: Duration = Duration::from_millis(1500);
const HIDDEN_TICK: Duration = Duration::from_secs(2);
const STRIP_RECENT: Duration = Duration::from_secs(4);

/// Where the last capture came from, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Stream,
    Screencapture,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Stream => "ScreenCaptureKit stream",
            Backend::Screencapture => "screencapture",
        }
    }
}

/// How long until the next look. Windows keeps its own fixed quarter second.
pub fn tick_every(stream_live: bool, window_shown: bool, strip_seen: Option<Instant>, now: Instant) -> Duration {
    if !window_shown {
        HIDDEN_TICK
    } else if stream_live || strip_seen.is_some_and(|at| now.saturating_duration_since(at) < STRIP_RECENT) {
        TICK
    } else {
        IDLE_TICK
    }
}

/// What a stream is opened for. When any of it changes, the stream is replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub window_id: u32,
    pub window_w: u32,
    pub window_h: u32,
    pub scale: u32,
    /// The part of the window the stream covers, from its top-left corner, in points.
    pub region_w: u32,
    pub region_h: u32,
}

impl Target {
    pub fn of(window: WindowFrame, region: Rect) -> Target {
        Target { window_id: window.id, window_w: window.rect.w, window_h: window.rect.h, scale: window.scale, region_w: region.w, region_h: region.h }
    }

    /// The frames' size, in pixels.
    pub fn pixels(&self) -> (usize, usize) {
        ((self.region_w * self.scale) as usize, (self.region_h * self.scale) as usize)
    }
}

/// `w` x `h` pixels from (x, y) of a BGRA image whose rows are `stride` bytes apart (wider
/// than its width when macOS pads them), cut down to what the image has.
#[allow(clippy::too_many_arguments)]
pub fn copy_region(px: &[u8], stride: usize, width: usize, height: usize, x: usize, y: usize, w: usize, h: usize) -> Image {
    let w = w.min(width.saturating_sub(x));
    let h = h.min(height.saturating_sub(y));
    let mut bgra = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let start = row * stride + x * 4;
        match px.get(start..start + w * 4) {
            Some(line) => bgra.extend_from_slice(line),
            None => return Image { width: w, height: (row - y), bgra },
        }
    }
    Image { bgra, width: w, height: h }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a stream's callbacks (on their own queue) leave for the capture thread.
#[derive(Default)]
pub struct Shared {
    newest: Mutex<Option<(Image, Instant)>>,
    stopped: Mutex<Option<String>>,
    frames: AtomicU32,
}

impl Shared {
    pub fn put(&self, image: Image, at: Instant) {
        *lock(&self.newest) = Some((image, at));
        self.frames.fetch_add(1, Ordering::Relaxed);
    }

    /// macOS stopped the stream; the first reason is kept.
    pub fn stop(&self, why: String) {
        lock(&self.stopped).get_or_insert(why);
    }

    fn has_frame(&self) -> bool {
        lock(&self.newest).is_some()
    }

    fn stopped(&self) -> Option<String> {
        lock(&self.stopped).clone()
    }

    fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Option<(Image, Instant)> {
        let newest = lock(&self.newest);
        let (image, at) = newest.as_ref()?;
        Some((copy_region(&image.bgra, image.width * 4, image.width, image.height, x, y, w, h), *at))
    }

    fn take_frames(&self) -> u32 {
        self.frames.swap(0, Ordering::Relaxed)
    }
}

/// A running stream: macOS's (`LiveStream`) or a test's.
pub trait Stream {
    fn shared(&self) -> &Shared;
}

#[derive(Clone, Debug, PartialEq)]
pub enum StartError {
    /// macOS won't list the windows or refuses the capture: Screen Recording is missing.
    Permission(String),
    /// No ScreenCaptureKit on this macOS: screencapture from now on.
    Unsupported(String),
    Failed(String),
}

/// Where a look comes from.
#[derive(Debug, PartialEq)]
pub enum Take {
    /// The newest frame's part asked for, and how long ago macOS sent it.
    Frame(Image, Duration),
    /// macOS refused the stream for want of permission.
    Refused,
    /// No stream frame: use screencapture.
    Fallback,
}

struct Live<S> {
    target: Target,
    stream: S,
    started: Instant,
    framed: bool,
}

/// Keeps one stream on the client's window, opening, replacing and stopping it as the window
/// comes, changes and goes, and backing off when it keeps failing.
pub struct Streams<S, F> {
    start: F,
    live: Option<Live<S>>,
    window: Option<WindowFrame>,
    /// The last target a start was tried for: a new one is tried at once, the same one only
    /// after the back-off.
    tried: Option<Target>,
    failures: u32,
    retry_at: Option<Instant>,
    refused: bool,
    unsupported: bool,
    /// The last reason for falling back that went to the log.
    told: Option<String>,
    told_streaming: bool,
}

/// The wait before the next start, after `failures` failures in a row.
pub fn backoff(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    (FIRST_RETRY * 2u32.pow(doublings)).min(LAST_RETRY)
}

impl<S: Stream, F: FnMut(Target, WindowFrame) -> Result<S, StartError>> Streams<S, F> {
    pub fn new(start: F) -> Self {
        Streams { start, live: None, window: None, tried: None, failures: 0, retry_at: None, refused: false, unsupported: false, told: None, told_streaming: false }
    }

    /// A stream is open (its frames may still be on the way).
    pub fn live(&self) -> bool {
        self.live.is_some()
    }

    /// Frames the stream sent since the last call.
    pub fn take_frames(&self) -> u32 {
        self.live.as_ref().map_or(0, |live| live.stream.shared().take_frames())
    }

    /// Brings the stream in line with the client's window on screen now (None: no window, or
    /// no client) and the part of it to cover. Returns lines for the log.
    pub fn update(&mut self, window: Option<WindowFrame>, region: Option<Rect>, now: Instant) -> Vec<String> {
        let mut log = Vec::new();
        self.window = window;
        let want = window.zip(region).map(|(window, region)| Target::of(window, region));
        if let Some(live) = &mut self.live {
            if !live.framed && live.stream.shared().has_frame() {
                live.framed = true;
                if !self.told_streaming {
                    self.told_streaming = true;
                    self.told = None;
                    let (w, h) = live.target.pixels();
                    log.push(format!("Reading the strip from a ScreenCaptureKit stream on the game's window ({w}x{h} px of its corner)."));
                }
            }
            let why = live.stream.shared().stopped().map(|error| format!("stopped: {error}")).or_else(|| {
                (!live.framed && now.saturating_duration_since(live.started) >= FIRST_FRAME_WITHIN).then(|| format!("sent no picture within {} s", FIRST_FRAME_WITHIN.as_secs()))
            });
            if let Some(why) = why {
                self.live = None;
                log.extend(self.failed(now, why));
            } else if live.framed && now.saturating_duration_since(live.started) >= PROVEN_AFTER {
                self.failures = 0;
            }
        }
        if self.live.as_ref().is_some_and(|live| Some(live.target) != want) {
            // Dropping it stops it.
            self.live = None;
        }
        let (Some(target), Some(window)) = (want, window) else {
            return log;
        };
        if self.live.is_some() || self.unsupported {
            return log;
        }
        if self.tried == Some(target) && self.retry_at.is_some_and(|at| now < at) {
            return log;
        }
        self.tried = Some(target);
        match (self.start)(target, window) {
            Ok(stream) => {
                self.refused = false;
                self.live = Some(Live { target, stream, started: now, framed: false });
            }
            Err(StartError::Permission(error)) => {
                // The Reader's report tells the player what to do; this says what macOS said.
                self.refused = true;
                self.failures += 1;
                self.retry_at = Some(now + backoff(self.failures));
                let why = format!("refused: {error}");
                if self.told.as_ref() != Some(&why) {
                    log.push(format!("ScreenCaptureKit {why}"));
                    self.told = Some(why);
                }
            }
            Err(StartError::Unsupported(error)) => {
                self.unsupported = true;
                log.push(format!("Reading the strip with screencapture: {error}."));
            }
            Err(StartError::Failed(error)) => log.extend(self.failed(now, error)),
        }
        log
    }

    fn failed(&mut self, now: Instant, why: String) -> Option<String> {
        self.refused = false;
        self.failures += 1;
        let wait = backoff(self.failures);
        self.retry_at = Some(now + wait);
        if self.told.as_ref() == Some(&why) {
            return None;
        }
        self.told = Some(why.clone());
        self.told_streaming = false;
        Some(format!("Reading the strip with screencapture for now: the ScreenCaptureKit stream {why}. Trying it again in {} s.", wait.as_secs()))
    }

    /// Where a look at `rect` (screen points, from the window's corner) comes from.
    pub fn take(&self, rect: Rect, now: Instant) -> Take {
        if let (Some(live), Some(window)) = (&self.live, self.window) {
            let scale = window.scale as usize;
            let offset = |at: i32, corner: i32| (at - corner).max(0) as usize * scale;
            let cropped = live.stream.shared().crop(offset(rect.x, window.rect.x), offset(rect.y, window.rect.y), rect.w as usize * scale, rect.h as usize * scale);
            if let Some((image, at)) = cropped {
                return Take::Frame(image, now.saturating_duration_since(at));
            }
        }
        if self.refused {
            Take::Refused
        } else {
            Take::Fallback
        }
    }
}

/// The Reader's captures: from the stream when it has a frame, else from `fallback`
/// (screencapture).
pub struct Capturer<S, F, G> {
    pub streams: Streams<S, F>,
    fallback: G,
    last: Option<(Backend, Option<Duration>)>,
}

impl<S, F, G> Capturer<S, F, G> {
    pub fn new(streams: Streams<S, F>, fallback: G) -> Self {
        Capturer { streams, fallback, last: None }
    }

    /// Where the last capture came from, and its frame's age when from the stream.
    pub fn last(&self) -> Option<(Backend, Option<Duration>)> {
        self.last
    }
}

impl<S: Stream, F: FnMut(Target, WindowFrame) -> Result<S, StartError>, G: Grab> Grab for Capturer<S, F, G> {
    fn grab(&mut self, rect: Rect) -> Result<Image, GrabError> {
        match self.streams.take(rect, Instant::now()) {
            Take::Frame(image, age) => {
                self.last = Some((Backend::Stream, Some(age)));
                Ok(image)
            }
            Take::Refused => {
                self.last = Some((Backend::Stream, None));
                Err(GrabError::Permission)
            }
            Take::Fallback => {
                self.last = Some((Backend::Screencapture, None));
                self.fallback.grab(rect)
            }
        }
    }
}

pub use live::{start, Filter};

/// The stream itself, through ScreenCaptureKit. Needs the window server and permission, so
/// only the ignored live tests run it.
mod live {
    use super::*;
    use block2::RcBlock;
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use dispatch2::{DispatchQueue, DispatchRetained};
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::{AnyClass, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_media::{CMSampleBuffer, CMTime};
    use objc2_core_video::{
        kCVPixelFormatType_32BGRA, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth,
        CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    };
    use objc2_foundation::{NSArray, NSError};
    use objc2_screen_capture_kit::{
        SCContentFilter, SCFrameStatus, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamFrameInfoStatus, SCStreamOutput, SCStreamOutputType, SCWindow,
    };
    use std::sync::mpsc;
    use std::sync::Arc;

    /// SCStreamErrorUserDeclined: no Screen Recording permission.
    const USER_DECLINED: isize = -3801;

    /// What the stream looks at. The window on its own is what the companion uses: it reads
    /// the game's pixels even with another window over the corner, and follows the window as
    /// it moves. The display with only that window in it is there to compare, in the live test.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Filter {
        Window,
        #[cfg_attr(not(test), allow(dead_code))]
        Display,
    }

    /// Lets a ScreenCaptureKit object cross from macOS's queue to the capture thread, which
    /// only hands it back to ScreenCaptureKit.
    struct Sent<T>(T);
    unsafe impl<T> Send for Sent<T> {}

    fn describe(error: &NSError) -> String {
        format!("{} (code {})", error.localizedDescription(), error.code())
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "T3WowCaptureStreamOutput"]
        #[ivars = Arc<Shared>]
        struct Output;

        unsafe impl NSObjectProtocol for Output {}

        unsafe impl SCStreamOutput for Output {
            #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
            fn did_output(&self, _stream: &SCStream, buffer: &CMSampleBuffer, kind: SCStreamOutputType) {
                if kind == SCStreamOutputType::Screen {
                    if let Some(image) = picture(buffer) {
                        self.ivars().put(image, Instant::now());
                    }
                }
            }
        }

        unsafe impl SCStreamDelegate for Output {
            #[unsafe(method(stream:didStopWithError:))]
            fn did_stop(&self, _stream: &SCStream, error: &NSError) {
                self.ivars().stop(describe(error));
            }
        }
    );

    impl Output {
        fn new(shared: Arc<Shared>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(shared);
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Only a complete frame has new pixels; the others say nothing changed, or that the
    /// window is blank or hidden for now.
    fn complete(buffer: &CMSampleBuffer) -> bool {
        let Some(attachments) = (unsafe { buffer.sample_attachments_array(false) }) else {
            return false;
        };
        let attachments: CFArray<CFDictionary<CFString, CFType>> = unsafe { CFArray::wrap_under_get_rule(objc2_core_foundation::CFRetained::as_ptr(&attachments).as_ptr() as CFArrayRef) };
        let key = unsafe { CFString::wrap_under_get_rule(SCStreamFrameInfoStatus as *const _ as CFStringRef) };
        attachments
            .get(0)
            .and_then(|info| info.find(&key).and_then(|status| status.downcast::<CFNumber>()).and_then(|status| status.to_i64()))
            .is_some_and(|status| status == SCFrameStatus::Complete.0 as i64)
    }

    /// The frame's pixels, copied out of macOS's buffer so it goes back to the stream.
    fn picture(buffer: &CMSampleBuffer) -> Option<Image> {
        if !complete(buffer) {
            return None;
        }
        let pixels = unsafe { buffer.image_buffer() }?;
        if CVPixelBufferGetPixelFormatType(&pixels) != kCVPixelFormatType_32BGRA {
            return None;
        }
        if unsafe { CVPixelBufferLockBaseAddress(&pixels, CVPixelBufferLockFlags::ReadOnly) } != 0 {
            return None;
        }
        let (width, height, stride) = (CVPixelBufferGetWidth(&pixels), CVPixelBufferGetHeight(&pixels), CVPixelBufferGetBytesPerRow(&pixels));
        let base = CVPixelBufferGetBaseAddress(&pixels);
        let image = (!base.is_null() && width > 0 && height > 0 && stride >= width * 4).then(|| {
            let bytes = unsafe { std::slice::from_raw_parts(base as *const u8, stride * (height - 1) + width * 4) };
            copy_region(bytes, stride, width, height, 0, 0, width, height)
        });
        unsafe { CVPixelBufferUnlockBaseAddress(&pixels, CVPixelBufferLockFlags::ReadOnly) };
        image
    }

    /// The windows and displays macOS lets this process capture.
    fn shareable_content() -> Result<Retained<SCShareableContent>, StartError> {
        let (tx, rx) = mpsc::channel::<Sent<Result<Retained<SCShareableContent>, String>>>();
        let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
            let result = match unsafe { Retained::retain(content) } {
                Some(content) => Ok(content),
                None => Err(unsafe { Retained::retain(error) }.map_or_else(|| "no answer".to_string(), |error| describe(&error))),
            };
            let _ = tx.send(Sent(result));
        });
        unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(true, false, &handler) };
        match rx.recv_timeout(ANSWER_WITHIN) {
            Ok(Sent(Ok(content))) => Ok(content),
            Ok(Sent(Err(error))) => Err(StartError::Permission(format!("macOS wouldn't list the windows to capture: {error}"))),
            Err(_) => Err(StartError::Failed(format!("got no list of windows from macOS within {} s", ANSWER_WITHIN.as_secs()))),
        }
    }

    /// A ScreenCaptureKit stream on the window's top-left corner. Dropping it stops it.
    pub struct LiveStream {
        shared: Arc<Shared>,
        stream: Retained<SCStream>,
        // The stream holds its output and delegate weakly.
        _output: Retained<Output>,
        _queue: DispatchRetained<DispatchQueue>,
    }

    impl Stream for LiveStream {
        fn shared(&self) -> &Shared {
            &self.shared
        }
    }

    impl Drop for LiveStream {
        fn drop(&mut self) {
            autoreleasepool(|_| {
                let (tx, rx) = mpsc::channel();
                let done = RcBlock::new(move |_error: *mut NSError| {
                    let _ = tx.send(());
                });
                unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&done)) };
                let _ = rx.recv_timeout(Duration::from_secs(1));
            });
        }
    }

    /// Opens a stream for `target` on `window` (both as just seen in the window list).
    pub fn start(target: Target, window: WindowFrame, filter: Filter) -> Result<LiveStream, StartError> {
        // Linked weakly (build.rs): before macOS 12.3 the classes aren't there.
        if AnyClass::get(c"SCStream").is_none() || AnyClass::get(c"SCShareableContent").is_none() {
            return Err(StartError::Unsupported("this macOS has no ScreenCaptureKit (it came with macOS 12.3)".to_string()));
        }
        autoreleasepool(|_| open(target, window, filter))
    }

    fn open(target: Target, window: WindowFrame, filter: Filter) -> Result<LiveStream, StartError> {
        let content = shareable_content()?;
        let windows = unsafe { content.windows() };
        let sc_window: Retained<SCWindow> = windows
            .iter()
            .find(|w| unsafe { w.windowID() } == target.window_id)
            .ok_or_else(|| StartError::Failed(format!("found no window {} among those macOS offers to capture", target.window_id)))?;
        let size = CGSize { width: f64::from(target.region_w), height: f64::from(target.region_h) };
        let (content_filter, origin) = match filter {
            // A window's own stream measures from its top-left corner.
            Filter::Window => (unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &sc_window) }, CGPoint { x: 0.0, y: 0.0 }),
            Filter::Display => {
                let corner = CGPoint { x: f64::from(window.rect.x), y: f64::from(window.rect.y) };
                let displays = unsafe { content.displays() };
                let display = displays
                    .iter()
                    .find(|d| {
                        let frame = unsafe { d.frame() };
                        corner.x >= frame.origin.x && corner.y >= frame.origin.y && corner.x < frame.origin.x + frame.size.width && corner.y < frame.origin.y + frame.size.height
                    })
                    .ok_or_else(|| StartError::Failed("found no display under the window's corner".to_string()))?;
                let frame = unsafe { display.frame() };
                let only = NSArray::from_retained_slice(std::slice::from_ref(&sc_window));
                let content_filter = unsafe { SCContentFilter::initWithDisplay_includingWindows(SCContentFilter::alloc(), &display, &only) };
                (content_filter, CGPoint { x: corner.x - frame.origin.x, y: corner.y - frame.origin.y })
            }
        };
        let (w_px, h_px) = target.pixels();
        let config = unsafe { SCStreamConfiguration::new() };
        unsafe {
            config.setWidth(w_px);
            config.setHeight(h_px);
            config.setSourceRect(CGRect { origin, size });
            config.setScalesToFit(false);
            config.setPixelFormat(kCVPixelFormatType_32BGRA);
            config.setShowsCursor(false);
            config.setMinimumFrameInterval(CMTime::new(1, FRAMES_PER_SECOND));
            config.setQueueDepth(QUEUE_DEPTH);
            // macOS 14 and later: the frame starts at the window's corner, not its shadow's.
            if config.respondsToSelector(sel!(setIgnoreShadowsSingleWindow:)) {
                config.setIgnoreShadowsSingleWindow(true);
            }
        }
        let shared = Arc::new(Shared::default());
        let output = Output::new(shared.clone());
        let delegate = ProtocolObject::<dyn SCStreamDelegate>::from_ref(&*output);
        let stream = unsafe { SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &content_filter, &config, Some(delegate)) };
        let queue = DispatchQueue::new("t3wow.capture.stream", None);
        unsafe { stream.addStreamOutput_type_sampleHandlerQueue_error(ProtocolObject::from_ref(&*output), SCStreamOutputType::Screen, Some(&queue)) }
            .map_err(|error| StartError::Failed(format!("wouldn't take an output: {}", describe(&error))))?;
        let (tx, rx) = mpsc::channel::<Option<(isize, String)>>();
        let done = RcBlock::new(move |error: *mut NSError| {
            let _ = tx.send(unsafe { Retained::retain(error) }.map(|error| (error.code(), describe(&error))));
        });
        unsafe { stream.startCaptureWithCompletionHandler(Some(&done)) };
        // Built first, so a start that fails or comes late is stopped when it drops.
        let live = LiveStream { shared, stream, _output: output, _queue: queue };
        match rx.recv_timeout(ANSWER_WITHIN) {
            Ok(None) => Ok(live),
            Ok(Some((USER_DECLINED, error))) => Err(StartError::Permission(error)),
            Ok(Some((_, error))) => Err(StartError::Failed(format!("didn't start: {error}"))),
            Err(_) => Err(StartError::Failed(format!("didn't start within {} s", ANSWER_WITHIN.as_secs()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    const WINDOW: WindowFrame = WindowFrame { rect: Rect { x: 100, y: 50, w: 1728, h: 1117 }, scale: 2, id: 77 };
    const REGION: Rect = Rect { x: 100, y: 50, w: 820, h: 204 };

    /// A BGRA image `w` x `h` with `pad` unused bytes after each row; each pixel says where it is.
    fn padded(w: usize, h: usize, pad: usize) -> (Vec<u8>, usize) {
        let stride = w * 4 + pad;
        let mut px = vec![0xeeu8; stride * h];
        for y in 0..h {
            for x in 0..w {
                px[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&[x as u8, y as u8, 7, 255]);
            }
        }
        (px, stride)
    }

    #[test]
    fn a_region_is_copied_from_padded_rows_and_cut_to_the_frame() {
        let (px, stride) = padded(10, 6, 24);
        let image = copy_region(&px, stride, 10, 6, 2, 1, 3, 2);
        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(image.bgra, vec![2, 1, 7, 255, 3, 1, 7, 255, 4, 1, 7, 255, 2, 2, 7, 255, 3, 2, 7, 255, 4, 2, 7, 255], "no padding bytes in the copy");
        let edge = copy_region(&px, stride, 10, 6, 8, 4, 50, 50);
        assert_eq!((edge.width, edge.height), (2, 2), "cut down to what the frame has");
        assert_eq!(&edge.bgra[..4], &[8, 4, 7, 255]);
        let outside = copy_region(&px, stride, 10, 6, 20, 20, 5, 5);
        assert_eq!((outside.width, outside.height, outside.bgra.len()), (0, 0, 0));
        // A buffer whose last row stops at the pixels, not the padding.
        let short = &px[..stride * 5 + 10 * 4];
        assert_eq!(copy_region(short, stride, 10, 6, 0, 0, 10, 6).height, 6);
    }

    #[test]
    fn a_strip_drawn_in_a_padded_frame_reads_back_through_the_stream() {
        use crate::codec::{decode_cells, encode_frame, magic_cells, CELLS_PER_ROW};
        let payload = r#"{"check":"T3 WoW ✓","q":[{"m":"through the stream","s":1,"t":"send"}]}"#;
        let cells = encode_frame(5, payload);
        let (w, h, cell, dy) = (1640, 160, 4, 56);
        let stride = w * 4 + 64;
        let mut px = vec![0x60u8; stride * h];
        for (i, &v) in cells.iter().enumerate() {
            let (x0, y0) = ((i % CELLS_PER_ROW) * cell, dy + (i / CELLS_PER_ROW) * cell);
            for y in y0..y0 + cell {
                for x in x0..x0 + cell {
                    let at = y * stride + x * 4;
                    px[at..at + 3].copy_from_slice(&[if v & 1 != 0 { 255 } else { 0 }, if v & 2 != 0 { 255 } else { 0 }, if v & 4 != 0 { 255 } else { 0 }]);
                }
            }
        }
        let shared = Arc::new(Shared::default());
        shared.put(copy_region(&px, stride, w, h, 0, 0, w, h), Instant::now());
        let (mut streams, _) = streams_over(vec![Ok(Fake(shared))]);
        let window = WindowFrame { rect: Rect { x: 0, y: 0, w: 1728, h: 1117 }, ..WINDOW };
        streams.update(Some(window), Some(Rect { x: 0, y: 0, w: 820, h: 80 }), Instant::now());
        let mut reader = super::super::mac::Reader::new(Capturer::new(streams, |_| -> Result<Image, GrabError> { panic!("no screencapture with a frame") }), magic_cells());
        let (read, _) = reader.read(Some(window), Instant::now());
        match read {
            super::super::mac::Read::Cells(read) => assert_eq!(decode_cells(&read).unwrap().payload, payload),
            other => panic!("{other:?}"),
        }
        assert_eq!(reader.grabber().last().map(|(backend, _)| backend), Some(Backend::Stream));
    }

    struct Fake(Arc<Shared>);

    impl Stream for Fake {
        fn shared(&self) -> &Shared {
            &self.0
        }
    }

    type Started = Rc<RefCell<Vec<Target>>>;
    type FakeStreams = Streams<Fake, Box<dyn FnMut(Target, WindowFrame) -> Result<Fake, StartError>>>;

    /// Streams whose starts give `results` in turn (then fail), recording each target.
    fn streams_over(results: Vec<Result<Fake, StartError>>) -> (FakeStreams, Started) {
        let started = Started::default();
        let seen = started.clone();
        let mut results = results.into_iter();
        let start: Box<dyn FnMut(Target, WindowFrame) -> Result<Fake, StartError>> = Box::new(move |target, _window| {
            seen.borrow_mut().push(target);
            results.next().unwrap_or_else(|| Err(StartError::Failed("no more".into())))
        });
        let streams = Streams::new(start);
        (streams, started)
    }

    fn fake() -> (Fake, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        (Fake(shared.clone()), shared)
    }

    fn picture(w: usize, h: usize) -> Image {
        let (px, stride) = padded(w, h, 0);
        copy_region(&px, stride, w, h, 0, 0, w, h)
    }

    #[test]
    fn one_stream_follows_the_window_and_a_new_window_gets_a_new_one_at_once() {
        let (first, first_shared) = fake();
        let (second, _) = fake();
        let (third, _) = fake();
        let (mut streams, started) = streams_over(vec![Ok(first), Ok(second), Ok(third)]);
        let t0 = Instant::now();
        streams.update(Some(WINDOW), Some(REGION), t0);
        assert!(streams.live());
        first_shared.put(picture(4, 4), t0);
        let log = streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_millis(250));
        assert_eq!(log.len(), 1, "the first frame is told once: {log:?}");
        // Moving the window keeps the stream: it follows the window.
        let moved = WindowFrame { rect: Rect { x: 400, y: 300, ..WINDOW.rect }, ..WINDOW };
        for i in 2..10 {
            assert!(streams.update(Some(moved), Some(REGION), t0 + Duration::from_millis(250) * i).is_empty());
        }
        assert_eq!(started.borrow().len(), 1);
        // A new window (fullscreen, a new client) or a new size: a new stream, at once.
        let other = WindowFrame { id: 78, ..WINDOW };
        streams.update(Some(other), Some(REGION), t0 + Duration::from_secs(3));
        let resized = WindowFrame { rect: Rect { w: 1280, h: 800, ..WINDOW.rect }, ..other };
        streams.update(Some(resized), Some(REGION), t0 + Duration::from_secs(3));
        let started = started.borrow();
        assert_eq!(started.iter().map(|t| (t.window_id, t.window_w)).collect::<Vec<_>>(), vec![(77, 1728), (78, 1728), (78, 1280)]);
        assert_eq!(started[0].pixels(), (1640, 408));
    }

    #[test]
    fn no_window_stops_the_stream_and_starts_none() {
        let (first, first_shared) = fake();
        let (mut streams, started) = streams_over(vec![Ok(first)]);
        let t0 = Instant::now();
        streams.update(Some(WINDOW), Some(REGION), t0);
        first_shared.put(picture(4, 4), t0);
        streams.update(None, None, t0 + Duration::from_secs(1));
        assert!(!streams.live());
        assert_eq!(streams.take(REGION, t0), Take::Fallback);
        for i in 2..20 {
            streams.update(None, None, t0 + Duration::from_secs(i));
        }
        assert_eq!(started.borrow().len(), 1);
        assert_eq!(Arc::strong_count(&first_shared), 1, "the stream was dropped, which stops it");
    }

    #[test]
    fn a_stopped_stream_falls_back_and_comes_back_after_a_growing_wait() {
        let (first, first_shared) = fake();
        let (mut streams, started) = streams_over(vec![Ok(first), Err(StartError::Failed("busy".into()))]);
        let t0 = Instant::now();
        streams.update(Some(WINDOW), Some(REGION), t0);
        first_shared.put(picture(4, 4), t0);
        streams.update(Some(WINDOW), Some(REGION), t0);
        // Sleep, or a display change: macOS stops the stream.
        first_shared.stop("The display went to sleep (code -3815)".into());
        let log = streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(10));
        assert!(log.iter().any(|l| l.contains("stopped: The display went to sleep") && l.contains("screencapture")), "{log:?}");
        assert!(!streams.live());
        assert_eq!(streams.take(REGION, t0), Take::Fallback);
        // One second later, then two after the next failure.
        streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_millis(10_900));
        assert_eq!(started.borrow().len(), 1, "not before the wait");
        let log = streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(11));
        assert_eq!(started.borrow().len(), 2);
        assert!(log.iter().any(|l| l.contains("busy")), "{log:?}");
        streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_millis(12_900));
        assert_eq!(started.borrow().len(), 2);
        streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(13));
        assert_eq!(started.borrow().len(), 3);
        let log = streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(17));
        assert!(log.is_empty(), "the same failure is told once: {log:?}");
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(4), Duration::from_secs(8));
        assert_eq!(backoff(40), Duration::from_secs(300));
    }

    #[test]
    fn a_stream_that_sends_nothing_has_failed() {
        let (first, _) = fake();
        let (mut streams, _) = streams_over(vec![Ok(first)]);
        let t0 = Instant::now();
        streams.update(Some(WINDOW), Some(REGION), t0);
        assert!(streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(2)).is_empty());
        assert!(streams.live(), "frames come only when the window changes, but the first comes at once");
        let log = streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(3));
        assert!(log.iter().any(|l| l.contains("sent no picture")), "{log:?}");
        assert!(!streams.live());
    }

    #[test]
    fn an_old_frame_is_still_the_window_and_is_cut_where_the_reader_asks() {
        let (first, first_shared) = fake();
        let (mut streams, _) = streams_over(vec![Ok(first)]);
        let t0 = Instant::now();
        streams.update(Some(WINDOW), Some(REGION), t0);
        assert_eq!(streams.take(REGION, t0), Take::Fallback, "no frame yet: screencapture");
        first_shared.put(picture(40, 30), t0);
        let later = t0 + Duration::from_secs(30);
        assert!(streams.update(Some(WINDOW), Some(REGION), later).len() <= 1);
        // 10 x 5 points from 2 x 1 points into the window, at scale 2.
        let Take::Frame(image, age) = streams.take(Rect { x: WINDOW.rect.x + 2, y: WINDOW.rect.y + 1, w: 10, h: 5 }, later) else { panic!("a frame") };
        assert_eq!(age, Duration::from_secs(30), "no new frame isn't a failure");
        assert_eq!((image.width, image.height), (20, 10));
        assert_eq!(&image.bgra[..4], &[4, 2, 7, 255]);
    }

    #[test]
    fn a_refused_stream_is_the_permission_state_and_no_screencapture_runs() {
        let (mut streams, started) = streams_over(vec![Err(StartError::Permission("The user declined TCCs (code -3801)".into()))]);
        let t0 = Instant::now();
        let log = streams.update(Some(WINDOW), Some(REGION), t0);
        assert!(log.iter().any(|l| l.contains("-3801")), "{log:?}");
        let mut capturer = Capturer::new(streams, |_| -> Result<Image, GrabError> { panic!("screencapture wouldn't be allowed either") });
        assert_eq!(capturer.grab(REGION), Err(GrabError::Permission));
        // Asked again after the wait, in case permission came.
        capturer.streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(1));
        assert_eq!(started.borrow().len(), 2);
    }

    #[test]
    fn without_screencapturekit_screencapture_does_every_capture() {
        let (mut streams, started) = streams_over(vec![Err(StartError::Unsupported("this macOS has no ScreenCaptureKit".into()))]);
        let t0 = Instant::now();
        let log = streams.update(Some(WINDOW), Some(REGION), t0);
        assert_eq!(log.len(), 1);
        for i in 1..30 {
            assert!(streams.update(Some(WINDOW), Some(REGION), t0 + Duration::from_secs(i * 60)).is_empty());
        }
        assert_eq!(started.borrow().len(), 1, "never tried again");
        let grabs = Rc::new(RefCell::new(0));
        let counted = grabs.clone();
        let mut capturer = Capturer::new(streams, move |_| {
            *counted.borrow_mut() += 1;
            Ok(picture(2, 2))
        });
        assert_eq!(capturer.grab(REGION), Ok(picture(2, 2)));
        assert_eq!(*grabs.borrow(), 1);
        assert_eq!(capturer.last(), Some((Backend::Screencapture, None)));
    }

    #[test]
    fn the_stream_serves_captures_once_it_has_a_frame() {
        let (first, first_shared) = fake();
        let (mut streams, _) = streams_over(vec![Ok(first)]);
        let t0 = Instant::now();
        streams.update(Some(WINDOW), Some(REGION), t0);
        let grabs = Rc::new(RefCell::new(0));
        let counted = grabs.clone();
        let mut capturer = Capturer::new(streams, move |_| {
            *counted.borrow_mut() += 1;
            Ok(picture(1, 1))
        });
        assert_eq!(capturer.grab(REGION).unwrap(), picture(1, 1), "screencapture until the first frame");
        assert_eq!(capturer.last(), Some((Backend::Screencapture, None)));
        first_shared.put(picture(8, 8), Instant::now());
        first_shared.put(picture(6, 6), Instant::now());
        let image = capturer.grab(Rect { w: 2, h: 2, ..REGION }).unwrap();
        assert_eq!((image.width, image.height), (4, 4), "from the newest frame");
        assert_eq!(*grabs.borrow(), 1);
        assert!(matches!(capturer.last(), Some((Backend::Stream, Some(_)))));
        assert_eq!(capturer.streams.take_frames(), 2);
        assert_eq!(capturer.streams.take_frames(), 0);
    }

    #[test]
    fn screencapture_looks_less_often_while_the_strip_hides() {
        let now = Instant::now();
        let ago = |s: u64| Some(now - Duration::from_secs(s));
        assert_eq!(tick_every(true, true, None, now), Duration::from_millis(250), "a stream: always four times a second");
        assert_eq!(tick_every(false, true, ago(1), now), Duration::from_millis(250), "the strip was just there");
        assert_eq!(tick_every(false, true, ago(4), now), Duration::from_millis(1500));
        assert_eq!(tick_every(false, true, None, now), Duration::from_millis(1500));
        assert_eq!(tick_every(false, false, ago(1), now), Duration::from_secs(2), "no window to look at");
        assert_eq!(tick_every(true, false, None, now), Duration::from_secs(2));
    }

    /// CPU seconds so far: macOS's systemstatusd (through ps, as it is root's) and this process.
    fn cpu_now() -> (Option<f64>, f64) {
        let systemstatusd = std::process::Command::new("/usr/bin/pgrep").args(["-x", "systemstatusd"]).output().ok().and_then(|out| {
            let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let out = std::process::Command::new("/bin/ps").args(["-o", "time=", "-p", &pid]).output().ok()?;
            let time = String::from_utf8_lossy(&out.stdout).trim().to_string();
            time.split(':').try_fold(0.0, |total, part| part.parse::<f64>().ok().map(|v| total * 60.0 + v))
        });
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        (systemstatusd, tv(usage.ru_utime) + tv(usage.ru_stime))
    }

    fn cpu_line(before: (Option<f64>, f64), secs: f64) -> String {
        let after = cpu_now();
        let systemstatusd = before.0.zip(after.0).map_or("unknown".to_string(), |(a, b)| format!("+{:.2} s", b - a));
        format!("over {secs:.0} s: systemstatusd {systemstatusd}, this process +{:.2} s of CPU", after.1 - before.1)
    }

    /// Streams a window for a while and says what each look cost. The running WoW client's
    /// window, or the largest window of T3WOW_STREAM_PID; T3WOW_STREAM_SECS (20), and
    /// T3WOW_STREAM_FILTER=display to compare the display filter. Run with
    /// `cargo test --manifest-path companion/Cargo.toml stream::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_stream_on_a_window() {
        use super::super::mac;
        let pid = std::env::var("T3WOW_STREAM_PID").ok().and_then(|p| p.parse().ok()).or_else(|| mac::wow_processes().first().map(|p| p.0));
        let Some(pid) = pid else {
            println!("no WoW client running and no T3WOW_STREAM_PID: nothing to stream");
            return;
        };
        let secs: u64 = std::env::var("T3WOW_STREAM_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
        let filter = if std::env::var("T3WOW_STREAM_FILTER").as_deref() == Ok("display") { Filter::Display } else { Filter::Window };
        let window = mac::window_of(pid).expect("a window on screen");
        let region = mac::stream_region(window);
        println!("window {window:?}, region {region:?}, filter {filter:?}");
        let opened = Instant::now();
        let mut streams = Streams::new(|target, window| start(target, window, filter));
        for line in streams.update(Some(window), Some(region), Instant::now()) {
            println!("{line}");
        }
        assert!(streams.live(), "the stream started");
        println!("start took {:.1} ms", opened.elapsed().as_secs_f64() * 1000.0);
        let mut first = None;
        let (mut takes, mut total, mut slowest, mut ages, mut frames) = (0u32, Duration::ZERO, Duration::ZERO, Duration::ZERO, 0u32);
        let cpu = cpu_now();
        let looped = Instant::now();
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            let now = Instant::now();
            for line in streams.update(Some(window), Some(region), now) {
                println!("{line}");
            }
            frames += streams.take_frames();
            let began = Instant::now();
            if let Take::Frame(image, age) = streams.take(region, now) {
                let took = began.elapsed();
                first.get_or_insert_with(|| (opened.elapsed(), image.clone()));
                takes += 1;
                total += took;
                slowest = slowest.max(took);
                ages += age;
            }
            std::thread::sleep(TICK);
        }
        println!("{}", cpu_line(cpu, looped.elapsed().as_secs_f64()));
        let (first_after, image) = first.expect("a frame from the stream");
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!(
            "first frame after {:.1} ms; {takes} looks, {:.3} ms on average, {:.3} ms at the slowest; {frames} frames; newest {:.0} ms old on average",
            ms(first_after),
            ms(total) / f64::from(takes.max(1)),
            ms(slowest),
            ms(ages) / f64::from(takes.max(1))
        );
        assert_eq!((image.width, image.height), (region.w as usize * window.scale as usize, region.h as usize * window.scale as usize), "the whole corner, in pixels");
        assert!(image.bgra.chunks(4).any(|p| p[..3] != [0, 0, 0]), "not an empty picture");
        // Against screencapture of the same corner: equal where nothing covers the window. The
        // stream reads the window itself, so a window in front changes only screencapture's.
        let covering = mac::windows_in_front(window.id, region);
        println!("windows in front over the corner: {covering:?}");
        if let Some((_, newest)) = streams.live.as_ref().and_then(|live| live.stream.shared().crop(0, 0, usize::MAX, usize::MAX)).map(|(i, at)| (at, i)) {
            if let Ok(shot) = mac::grab(region) {
                // The share of matching pixels with the stream's frame moved by (dx, dy), on a grid.
                let same = |dx: isize, dy: isize, step: usize| {
                    let (mut same, mut all) = (0usize, 0usize);
                    for y in (0..newest.height).step_by(step) {
                        for x in (0..newest.width).step_by(step) {
                            let (sx, sy) = (x as isize + dx, y as isize + dy);
                            if sx < 0 || sy < 0 || sx as usize >= shot.width || sy as usize >= shot.height {
                                continue;
                            }
                            let a = &newest.bgra[(y * newest.width + x) * 4..][..3];
                            let b = &shot.bgra[(sy as usize * shot.width + sx as usize) * 4..][..3];
                            all += 1;
                            same += a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 24) as usize;
                        }
                    }
                    same as f64 * 100.0 / all.max(1) as f64
                };
                println!("screencapture {}x{}: {:.1}% of pixels the same as the stream's newest frame, in place", shot.width, shot.height, same(0, 0, 1));
                let mut best = (0, 0, 0.0);
                for dy in -40isize..=40 {
                    for dx in -40isize..=40 {
                        let score = same(dx, dy, 7);
                        if score > best.2 {
                            best = (dx, dy, score);
                        }
                    }
                }
                println!("best match moved by ({}, {}) px: {:.1}%", best.0, best.1, best.2);
                // What the strip decoder sees: each channel above 127 or not.
                let (w, h) = (newest.width.min(shot.width), newest.height.min(shot.height));
                let (mut bits_same, mut diff_total, mut box_) = (0usize, 0u64, (usize::MAX, usize::MAX, 0usize, 0usize));
                for y in 0..h {
                    for x in 0..w {
                        let a = &newest.bgra[(y * newest.width + x) * 4..][..3];
                        let b = &shot.bgra[(y * shot.width + x) * 4..][..3];
                        let bits = |p: &[u8]| p.iter().map(|v| (*v > 127) as u8).collect::<Vec<_>>();
                        bits_same += (bits(a) == bits(b)) as usize;
                        diff_total += a.iter().zip(b).map(|(a, b)| u64::from(a.abs_diff(*b))).sum::<u64>();
                        if !a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 24) {
                            box_ = (box_.0.min(x), box_.1.min(y), box_.2.max(x), box_.3.max(y));
                        }
                    }
                }
                if let Some(dir) = std::env::var_os("T3WOW_STREAM_PNG_DIR") {
                    // The top-left 480 x 160 px of each, side by side: stream left, screencapture right.
                    let (cw, ch) = (480.min(w), 160.min(h));
                    let mut rgba = Vec::with_capacity(cw * 2 * ch * 4);
                    for y in 0..ch {
                        for (image, iw) in [(&newest.bgra, newest.width), (&shot.bgra, shot.width)] {
                            for x in 0..cw {
                                let p = &image[(y * iw + x) * 4..][..3];
                                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
                            }
                        }
                    }
                    let path = std::path::Path::new(&dir).join(format!("corner-{}.png", window.id));
                    let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), (cw * 2) as u32, ch as u32);
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
                    println!("corners written to {}", path.display());
                }
                println!(
                    "threshold bits the same: {:.2}%; mean difference {:.1} per channel; differing pixels within x {}..{}, y {}..{}",
                    bits_same as f64 * 100.0 / (w * h) as f64,
                    diff_total as f64 / (w * h * 3) as f64,
                    box_.0,
                    box_.2,
                    box_.1,
                    box_.3
                );
                // Where nothing is in front, the two agree in place.
                if covering.is_empty() {
                    assert!(same(0, 0, 1) > 90.0, "an uncovered window reads the same both ways");
                }
            }
        }
    }

    /// The same looks with screencapture, four times a second, to compare.
    #[test]
    #[ignore]
    fn live_screencapture_on_a_window() {
        use super::super::mac;
        let pid = std::env::var("T3WOW_STREAM_PID").ok().and_then(|p| p.parse().ok()).or_else(|| mac::wow_processes().first().map(|p| p.0));
        let Some(pid) = pid else {
            println!("no WoW client running and no T3WOW_STREAM_PID: nothing to capture");
            return;
        };
        let secs: u64 = std::env::var("T3WOW_STREAM_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
        let window = mac::window_of(pid).expect("a window on screen");
        let region = mac::stream_region(window);
        let (mut grabs, mut total, mut slowest) = (0u32, Duration::ZERO, Duration::ZERO);
        let cpu = cpu_now();
        let looped = Instant::now();
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            let began = Instant::now();
            mac::grab(region).expect("a capture");
            let took = began.elapsed();
            grabs += 1;
            total += took;
            slowest = slowest.max(took);
            std::thread::sleep(TICK.saturating_sub(took));
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!("{}", cpu_line(cpu, looped.elapsed().as_secs_f64()));
        println!("{grabs} screencaptures of {region:?}, {:.1} ms on average, {:.1} ms at the slowest", ms(total) / f64::from(grabs), ms(slowest));
    }
}
