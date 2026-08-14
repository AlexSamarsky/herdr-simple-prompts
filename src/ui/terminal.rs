use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use std::io::{self, Stdout, Write};

/// Alternate scroll: the terminal translates the wheel into arrow keys while an
/// application holds the alternate screen.
///
/// It is not mouse reporting, so native text selection keeps working — the
/// overlay deliberately never grabs the mouse. Without this the wheel reaches
/// nobody: the alternate screen has no scrollback for the multiplexer to move.
const ENABLE_ALTERNATE_SCROLL: &str = "\u{1b}[?1007h";
const DISABLE_ALTERNATE_SCROLL: &str = "\u{1b}[?1007l";
use std::sync::Once;

static PANIC_HOOK: Once = Once::new();

pub struct TerminalGuard;

impl TerminalGuard {
    pub fn enter(stdout: &mut Stdout) -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let entered = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, Hide)
            .and_then(|()| stdout.write_all(ENABLE_ALTERNATE_SCROLL.as_bytes()))
            .and_then(|()| stdout.flush());
        if let Err(error) = entered {
            let _ = write_restore_sequences(stdout);
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous(info);
        }));
    });
}

fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = write_restore_sequences(&mut stdout);
    let _ = disable_raw_mode();
}

fn write_restore_sequences<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(DISABLE_ALTERNATE_SCROLL.as_bytes())?;
    execute!(writer, Show, DisableBracketedPaste, LeaveAlternateScreen)
}

#[cfg(test)]
mod tests {
    use super::{TerminalGuard, write_restore_sequences};
    use std::io;
    use std::process::Command;

    #[test]
    fn restoration_sequence_leaves_alternate_screen_and_restores_input_modes() {
        let mut output = Vec::new();

        write_restore_sequences(&mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\u{1b}[?25h"));
        assert!(output.contains("\u{1b}[?1007l"));
        assert!(!output.contains("\u{1b}[?1000l"));
        assert!(output.contains("\u{1b}[?2004l"));
        assert!(output.contains("\u{1b}[?1049l"));
    }

    #[test]
    fn panic_restores_terminal_in_a_pty() {
        const CHILD: &str = "HERDR_SIMPLE_PROMPTS_PANIC_PTY_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let mut stdout = io::stdout();
            let _guard = TerminalGuard::enter(&mut stdout).unwrap();
            panic!("intentional terminal restoration probe");
        }

        let executable = std::env::current_exe().unwrap();
        let test_name = "ui::terminal::tests::panic_restores_terminal_in_a_pty";
        #[cfg(target_os = "macos")]
        let output = Command::new("script")
            .args(["-q", "/dev/null", "env", &format!("{CHILD}=1")])
            .arg(&executable)
            .args(["--exact", test_name, "--nocapture"])
            .output()
            .unwrap();
        #[cfg(target_os = "linux")]
        let output = {
            let command = format!(
                "{CHILD}=1 {} --exact {test_name} --nocapture",
                executable.display()
            );
            Command::new("script")
                .args(["-q", "-e", "-c", &command, "/dev/null"])
                .output()
                .unwrap()
        };

        let mut transcript = output.stdout;
        transcript.extend(output.stderr);
        let transcript = String::from_utf8_lossy(&transcript);
        assert!(transcript.contains("\u{1b}[?2004h"), "{transcript}");
        assert!(transcript.contains("\u{1b}[?1049h"), "{transcript}");
        assert!(transcript.contains("\u{1b}[?1007h"), "{transcript}");
        assert!(!transcript.contains("\u{1b}[?1000h"), "{transcript}");
        assert!(!transcript.contains("\u{1b}[?1002h"), "{transcript}");
        assert!(!transcript.contains("\u{1b}[?1003h"), "{transcript}");
        assert!(!transcript.contains("\u{1b}[?1006h"), "{transcript}");
        assert!(transcript.contains("\u{1b}[?2004l"), "{transcript}");
        assert!(transcript.contains("\u{1b}[?1049l"), "{transcript}");
        assert!(transcript.contains("\u{1b}[?1007l"), "{transcript}");
    }
}
