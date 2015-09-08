extern crate gag;
extern crate libc;
extern crate num;
extern crate time;
extern crate termbox_sys as termbox;
#[macro_use] extern crate bitflags;

pub use self::style::{Style, RB_BOLD, RB_UNDERLINE, RB_REVERSE, RB_NORMAL};

use std::error::Error;
use std::fmt;
use std::io;
use std::char;
use std::default::Default;
use std::marker::PhantomData;

use num::FromPrimitive;
use termbox::RawEvent;
use libc::c_int;
use gag::Hold;
use time::Duration;

pub mod keyboard;
pub mod mouse;

pub use self::running::running;
pub use keyboard::Key;
pub use mouse::Mouse;

#[derive(Clone, Copy)]
/// Dictates the type of an event that has been recieved.
pub enum Event {
    /// A raw, non-wrapped key event
    KeyEventRaw(u8, u16, u32),
    /// A key event with the key code and information transformed into an optional `Key`
    KeyEvent(Option<Key>),
    /// A window resize event, with the new width and height
    ResizeEvent(i32, i32),
    /// A mouse event, with the type of the event, and the x and y coordinates
    MouseEvent(Mouse, i32, i32),
    /// An empty event
    NoEvent
}

#[derive(Clone, Copy, Debug)]
/// The mode of the input
pub enum InputMode {
    Current = 0x00,
    /// When ESC sequence is in the buffer and it doesn't match any known
    /// ESC sequence => ESC means TB_KEY_ESC
    Esc     = 0x01,
    /// When ESC sequence is in the buffer and it doesn't match any known
    /// sequence => ESC enables TB_MOD_ALT modifier for the next keyboard event.
    Alt     = 0x02,
    /// Same as `Esc` but enables mouse events
    EscMouse = 0x05,
    /// Same as `Alt` but enables mouse events
    AltMouse = 0x06
}

#[derive(Clone, Copy, PartialEq)]
#[repr(C,u16)]
/// The supported colors for Rustbox
pub enum Color {
    Default =  0x00,
    Black =    0x01,
    Red =      0x02,
    Green =    0x03,
    Yellow =   0x04,
    Blue =     0x05,
    Magenta =  0x06,
    Cyan =     0x07,
    White =    0x08,
}

mod style {
    bitflags! {
        #[repr(C)]
        /// The different styles that you can print text with.
        flags Style: u16 {
            /// The default color for the user's terminal emulator (TermBox)
            const TB_NORMAL_COLOR = 0x000F,
            /// Sets text to bold (or an equivilant) in the user's terminal emulator
            const RB_BOLD = 0x0100,
            /// Underlines text in the user's terminal emulator
            const RB_UNDERLINE = 0x0200,
            /// Reverses text in the user's terminal emulator
            const RB_REVERSE = 0x0400,
            /// The default color for the user's terminal emulator (RustBox)
            const RB_NORMAL = 0x0000,
            const TB_ATTRIB = RB_BOLD.bits | RB_UNDERLINE.bits | RB_REVERSE.bits,
        }
    }

    impl Style {
        /// Converts a `Color` to a `Style` (`u64`)
        pub fn from_color(color: super::Color) -> Style {
            Style { bits: color as u16 & TB_NORMAL_COLOR.bits }
        }
    }
}

/// An empty raw event
const NIL_RAW_EVENT: RawEvent = RawEvent { etype: 0, emod: 0, key: 0, ch: 0, w: 0, h: 0, x: 0, y: 0 };

#[derive(Debug)]
/// The varius types of errors that can happen with a Rustbox event.
pub enum EventError {
    /// Termbox had an error
    TermboxError,
    /// An unknown event occured
    Unknown(isize),
}

impl fmt::Display for EventError {
   fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
      write!(fmt, "{}", self.description())
   }
}

impl Error for EventError {
   fn description(&self) -> &str {
      match *self {
         EventError::TermboxError => "Error in Termbox",
         // I don't know how to format this without lifetime error.
         // EventError::Unknown(n) => &format!("There was an unknown error. Error code: {}", n),
         EventError::Unknown(_) => "Unknown error in Termbox",
      }
   }
}

impl FromPrimitive for EventError {
   fn from_i64(n: i64) -> Option<EventError> {
      match n {
         -1 => Some(EventError::TermboxError),
         n => Some(EventError::Unknown(n as isize)),
      }
   }

   fn from_u64(n: u64) -> Option<EventError> {
      Some(EventError::Unknown(n as isize))
   }
}

/// Wraper type for an `Event`, or an `EventError`.
pub type EventResult = Result<Event, EventError>;

/// Unpack a RawEvent to an Event
///
/// if the `raw` parameter is true, then the Event variant will be the raw
/// representation of the event.
///     for instance KeyEventRaw instead of KeyEvent
///
/// This is useful if you want to interpret the raw event data yourself, rather
/// than having rustbox translate it to its own representation.
fn unpack_event(ev_type: c_int, ev: &RawEvent, raw: bool) -> EventResult {
    match ev_type {
        0 => Ok(Event::NoEvent),
        1 => Ok(
            if raw {
                Event::KeyEventRaw(ev.emod, ev.key, ev.ch)
            } else {
                let k = match ev.key {
                    0 => char::from_u32(ev.ch).map(|c| Key::Char(c)),
                    a => Key::from_code(a),
                };
                Event::KeyEvent(k)
            }),
        2 => Ok(Event::ResizeEvent(ev.w, ev.h)),
        3 => {
            let mouse = Mouse::from_code(ev.key).unwrap_or(Mouse::Left);
            Ok(Event::MouseEvent(mouse, ev.x, ev.y))
        },
        // `unwrap` is safe here because FromPrimitive for EventError only returns `Some`.
        n => Err(FromPrimitive::from_isize(n as isize).unwrap()),
    }
}

#[derive(Debug)]
/// Represents the kinds of errors that can occur when initializing Rustbox.
pub enum InitError {
    /// Rustbox failded to connect to a stderr buffer
    BufferStderrFailed(io::Error),
    /// Rustbox is already open
    AlreadyOpen,
    /// Rustbox doesn't know how to deal with the terminal the user is running
    UnsupportedTerminal,
    /// Rustbox failed to open a TTY connection
    FailedToOpenTTy,
    /// An error with the pipe trap
    PipeTrapError,
    /// An unknown error occured
    Unknown(isize),
}

impl fmt::Display for InitError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "{}", self.description())
    }
}

impl Error for InitError {
    /// Returnes a textual description of the error.
    fn description(&self) -> &str {
        match *self {
            InitError::BufferStderrFailed(_) => "Could not redirect stderr",
            InitError::AlreadyOpen => "RustBox is already open",
            InitError::UnsupportedTerminal => "Unsupported terminal",
            InitError::FailedToOpenTTy => "Failed to open TTY",
            InitError::PipeTrapError => "Pipe trap error",
            InitError::Unknown(_) => "Unknown error from Termbox",
        }
    }

    fn cause(&self) -> Option<&Error> {
        match *self {
            InitError::BufferStderrFailed(ref e) => Some(e),
            _ => None
        }
    }
}

impl FromPrimitive for InitError {
   fn from_i64(n: i64) -> Option<InitError> {
      match n {
         -1 => Some(InitError::UnsupportedTerminal),
         -2 => Some(InitError::FailedToOpenTTy),
         -3 => Some(InitError::PipeTrapError),
         n => Some(InitError::Unknown(n as isize)),
      }
   }

   fn from_u64(n: u64) -> Option<InitError> {
      Some(InitError::Unknown(n as isize))
   }
}

#[allow(missing_copy_implementations)]
/// The main structure that represents Rustbox
pub struct RustBox {
    // We only bother to redirect stderr for the moment, since it's used for panic!
    _stderr: Option<Hold>,
    // RAII lock.
    //
    // Note that running *MUST* be the last field in the destructor, since destructors run in
    // top-down order. Otherwise it will not properly protect the above fields.
    _running: running::RunningGuard,
    // Termbox is not thread safe. See #39.
    _phantom: PhantomData<*mut ()>,
}

#[derive(Clone, Copy,Debug)]
/// The initial options that you can use to start a Rustbox instance.
pub struct InitOptions {
    /// Use this option to initialize with a specific input mode
    ///
    /// See InputMode enum for details on the variants.
    pub input_mode: InputMode,

    /// Use this option to automatically buffer stderr while RustBox is running.  It will be
    /// written when RustBox exits.
    ///
    /// This option uses a nonblocking OS pipe to buffer stderr output.  This means that if the
    /// pipe fills up, subsequent writes will fail until RustBox exits.  If this is a concern for
    /// your program, don't use RustBox's default pipe-based redirection; instead, redirect stderr
    /// to a log file or another process that is capable of handling it better.
    pub buffer_stderr: bool,
}

impl Default for InitOptions {
    /// Default options.
    fn default() -> Self {
        InitOptions {
            input_mode: InputMode::Current,
            buffer_stderr: false,
        }
    }
}

mod running {
    use std::sync::atomic::{self, AtomicBool};

    // The state of the RustBox is protected by the lock. Yay, global state!
    static RUSTBOX_RUNNING: AtomicBool = atomic::ATOMIC_BOOL_INIT;

    /// True if Rustbox is currently running. **Note**: Beware of races here -- don't rely on this for anything
    /// critical unless you happen to know that Rustbox cannot change state when it is called (a good
    /// usecase would be checking to see if it's worth risking double printing backtraces to avoid
    /// having them swallowed up by Rustbox).
    pub fn running() -> bool {
        RUSTBOX_RUNNING.load(atomic::Ordering::SeqCst)
    }

    // Internal RAII guard used to ensure we release the running lock whenever we acquire it.
    #[allow(missing_copy_implementations)]
    pub struct RunningGuard(());

    /// Creates a lock necissary for Rustbox to run
    pub fn run() -> Option<RunningGuard> {
        // Ensure that we are not already running and simultaneously set RUSTBOX_RUNNING using an
        // atomic swap. This ensures that contending threads don't trample each other.
        if RUSTBOX_RUNNING.swap(true, atomic::Ordering::SeqCst) {
            // The Rustbox was already running.
            None
        } else {
            // The RustBox was not already running, and now we have the lock.
            Some(RunningGuard(()))
        }
    }

    impl Drop for RunningGuard {
        fn drop(&mut self) {
            // Indicate that we're free now. We could probably get away with lower atomicity here,
            // but there's no reason to take that chance.
            RUSTBOX_RUNNING.store(false, atomic::Ordering::SeqCst);
        }
    }
}

impl RustBox {
    /// Initialize Rustbox.
    ///
    /// For the default options, you can use:
    ///
    /// ```
    /// use rustbox::RustBox;
    /// use std::default::Default;
    /// let rb = RustBox::init(Default::default());
    /// ```
    ///
    /// Otherwise, you can specify:
    ///
    /// ```
    /// use rustbox::{RustBox, InitOptions};
    /// use std::default::Default;
    /// let rb = RustBox::init(InitOptions { input_mode: rustbox::InputMode::Esc, ..Default::default() });
    /// ```
    pub fn init(opts: InitOptions) -> Result<RustBox, InitError> {
        let running = match running::run() {
            Some(r) => r,
            None => return Err(InitError::AlreadyOpen),
        };

        let stderr = if opts.buffer_stderr {
            Some(try!(Hold::stderr().map_err(|e| InitError::BufferStderrFailed(e))))
        } else {
            None
        };

        // Create the RustBox.
        let rb = unsafe { match termbox::tb_init() {
            0 => RustBox {
                _stderr: stderr,
                _running: running,
                _phantom: PhantomData,
            },
            res => {
                return Err(FromPrimitive::from_isize(res as isize).unwrap())
            }
        }};
        match opts.input_mode {
            InputMode::Current => (),
            _ => rb.set_input_mode(opts.input_mode),
        }
        Ok(rb)
    }

    /// Returns the width of the terminal emulator's screen.
    pub fn width(&self) -> usize {
        unsafe { termbox::tb_width() as usize }
    }

    /// Returns the height of the terminal emulator's screen.
    pub fn height(&self) -> usize {
        unsafe { termbox::tb_height() as usize }
    }

    /// Clears the terminal emulator's screen completely.
    pub fn clear(&self) {
        unsafe { termbox::tb_clear() }
    }

    /// Presents all the changes made to the screen all at once. In Rustbox, all the changes that are made to the screen are buffered, and this displays them.
    pub fn present(&self) {
        unsafe { termbox::tb_present() }
    }

    /// Changes the position of the user's cursor.
    pub fn set_cursor(&self, x: isize, y: isize) {
        unsafe { termbox::tb_set_cursor(x as c_int, y as c_int) }
    }

    /// Changes a specific cell on the screen at x and y, to a specific character, and forground and background.
    pub unsafe fn change_cell(&self, x: usize, y: usize, ch: u32, fg: u16, bg: u16) {
        termbox::tb_change_cell(x as c_int, y as c_int, ch, fg, bg)
    }

    /// Prints a string-slice to the screen at x and y, with a style, foreground, and background.
    pub fn print(&self, x: usize, y: usize, sty: Style, fg: Color, bg: Color, s: &str) {
        let fg = Style::from_color(fg) | (sty & style::TB_ATTRIB);
        let bg = Style::from_color(bg);
        for (i, ch) in s.chars().enumerate() {
            unsafe {
                self.change_cell(x+i, y, ch as u32, fg.bits(), bg.bits());
            }
        }
    }

    /// Same as `print` but a single character instead of an entire string.
    pub fn print_char(&self, x: usize, y: usize, sty: Style, fg: Color, bg: Color, ch: char) {
        let fg = Style::from_color(fg) | (sty & style::TB_ATTRIB);
        let bg = Style::from_color(bg);
        unsafe {
            self.change_cell(x, y, ch as u32, fg.bits(), bg.bits());
        }
    }

    /// Asks Rustbox if there is an event, and if there is, returns it.
    pub fn poll_event(&self, raw: bool) -> EventResult {
        let mut ev = NIL_RAW_EVENT;
        let rc = unsafe {
            termbox::tb_poll_event(&mut ev)
        };
        unpack_event(rc, &ev, raw)
    }

    /// Waits a certain amount of time before performing a `poll`.
    pub fn peek_event(&self, timeout: Duration, raw: bool) -> EventResult {
        let mut ev = NIL_RAW_EVENT;
        let rc = unsafe {
            termbox::tb_peek_event(&mut ev, timeout.num_milliseconds() as c_int)
        };
        unpack_event(rc, &ev, raw)
    }

    /// Changes the input mode.
    pub fn set_input_mode(&self, mode: InputMode) {
        unsafe {
            termbox::tb_select_input_mode(mode as c_int);
        }
    }
}

impl Drop for RustBox {
    /// Shuts down a Rustbox instance.
    fn drop(&mut self) {
        // Since only one instance of the RustBox is ever accessible, we should not
        // need to do this atomically.
        // Note: we should definitely have RUSTBOX_RUNNING = true here.
        unsafe {
            termbox::tb_shutdown();
        }
    }
}
