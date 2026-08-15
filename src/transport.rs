use crate::agent::{AgentIdentity, AgentKind, AgentStatus, agent_identity};
use crate::ansi::sanitize_ansi;
use crate::composer::{ComposerAccess, classify_native_composer, native_composer_content};
use crate::herdr::HerdrClient;
use crate::{AppError, AppResult};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

pub struct AgentTransport {
    client: HerdrClient,
    original: AgentIdentity,
}

impl AgentTransport {
    pub fn new(client: HerdrClient, original: AgentIdentity) -> Self {
        Self { client, original }
    }

    pub fn identity(&self) -> &AgentIdentity {
        &self.original
    }

    pub fn submit(&self, text: &str, expected_attachments: usize) -> AppResult<()> {
        self.validate_source()?;
        let ansi = self
            .client
            .pane_read_visible_ansi(&self.original.pane_id, 200)
            .map_err(|_| {
                AppError::new(
                    "send prompt",
                    "cannot verify native composer is safe to submit; prefix+m to return",
                )
            })?;
        let surface = sanitize_ansi(&ansi);
        match classify_native_composer(self.original.kind, &surface).access(expected_attachments) {
            ComposerAccess::Ready => {}
            ComposerAccess::Occupied => {
                return Err(AppError::new(
                    "send prompt",
                    "native composer contains unsent input; prefix+m to return",
                ));
            }
            ComposerAccess::Unknown => {
                return Err(AppError::new(
                    "send prompt",
                    "cannot verify native composer is safe to submit; prefix+m to return",
                ));
            }
        }
        self.client
            .agent_prompt(&self.original.pane_id, text)
            .map_err(|error| AppError::new("send prompt", error.to_string()))?;
        Ok(())
    }

    pub fn interrupt(&self) -> AppResult<()> {
        let current = self.validate_source()?;
        if !current.status.is_working() {
            return Err(AppError::new("interrupt", "agent is not working"));
        }
        self.client
            .agent_send_keys(&self.original.pane_id, &["esc"])
            .map_err(|error| AppError::new("interrupt", error.to_string()))?;
        Ok(())
    }

    pub fn forward_interaction_text(&self, text: &str) -> AppResult<()> {
        self.validate_blocked_source()?;
        self.client
            .pane_send_input(&self.original.pane_id, Some(text), &[])
            .map_err(|error| AppError::new("native interaction", error.to_string()))?;
        Ok(())
    }

    pub fn forward_interaction_key(&self, key: &str) -> AppResult<()> {
        self.validate_blocked_source()?;
        self.client
            .pane_send_input(&self.original.pane_id, None, &[key])
            .map_err(|error| AppError::new("native interaction", error.to_string()))?;
        Ok(())
    }

    /// Pastes the clipboard image and reports the number the pane gave it.
    ///
    /// The number has to come from the pane: it is a session counter there, and
    /// a guess made here would name a different picture — which is the one the
    /// overlay would later ask to have removed.
    pub fn forward_local_image_paste(&self) -> AppResult<usize> {
        let before = self.image_markers()?;
        self.validate_source()?;
        self.client
            .agent_send_keys(&self.original.pane_id, &["ctrl+v"])
            .map_err(|error| AppError::new("image paste", error.to_string()))?;
        self.verify_new_image_marker(&before)
    }

    pub fn forward_staged_image(&self, path: &Path) -> AppResult<usize> {
        let text = path
            .to_str()
            .ok_or_else(|| AppError::new("image paste", "image path is not UTF-8"))?;
        let before = self.image_markers()?;
        self.validate_source()?;
        self.client
            .pane_send_input(&self.original.pane_id, Some(text), &[])
            .map_err(|error| AppError::new("image paste", error.to_string()))?;
        self.verify_new_image_marker(&before)
    }

    pub fn remove_attachment(&self, marker: usize) -> AppResult<()> {
        self.validate_source()?;
        let removed = remove_native_attachment(
            self.original.kind,
            marker,
            || {
                self.client
                    .pane_read_visible_ansi(&self.original.pane_id, 200)
                    .map_err(|error| AppError::new("remove image", error.to_string()))
            },
            |keys, text| {
                self.client
                    .pane_send_input(&self.original.pane_id, text, keys)
                    .map(|_| ())
                    .map_err(|error| AppError::new("remove image", error.to_string()))
            },
        )?;
        if removed {
            Ok(())
        } else {
            Err(AppError::new(
                "remove image",
                "could not reach the image in the native composer; prefix+m to return",
            ))
        }
    }

    pub fn visible_source(&self, lines: u16) -> AppResult<String> {
        self.validate_source()?;
        self.client
            .pane_read_visible_text(&self.original.pane_id, u32::from(lines))
            .map_err(|error| AppError::new("source screen", error.to_string()))
    }

    pub fn recent_unwrapped_ansi(&self, lines: u32) -> AppResult<String> {
        self.validate_source()?;
        self.client
            .agent_read_recent_unwrapped_ansi(&self.original.pane_id, lines)
            .map_err(|error| AppError::new("capture final style", error.to_string()))
    }

    pub fn visible_source_ansi(&self, lines: u32) -> AppResult<String> {
        self.validate_source()?;
        self.read_visible_source_ansi(lines)
    }

    pub(crate) fn read_visible_source_ansi(&self, lines: u32) -> AppResult<String> {
        self.client
            .pane_read_visible_ansi(&self.original.pane_id, lines)
            .map_err(|error| AppError::new("source screen", error.to_string()))
    }

    pub fn refresh_identity(&self) -> AppResult<AgentIdentity> {
        self.validate_source()
    }

    fn validate_source(&self) -> AppResult<AgentIdentity> {
        let current = agent_identity(&self.client, &self.original.pane_id)?;
        if current.kind != self.original.kind || current.session_id != self.original.session_id {
            return Err(AppError::new(
                "agent",
                "source agent session changed; reopen Simple Prompts",
            ));
        }
        Ok(current)
    }

    fn validate_blocked_source(&self) -> AppResult<AgentIdentity> {
        let current = self.validate_source()?;
        if current.status != AgentStatus::Blocked {
            return Err(AppError::new(
                "native interaction",
                "source agent is no longer blocked",
            ));
        }
        Ok(current)
    }

    fn image_markers(&self) -> AppResult<Vec<usize>> {
        Ok(composer_markers(
            self.original.kind,
            &self.read_visible_source_ansi(200)?,
        ))
    }

    fn verify_new_image_marker(&self, before: &[usize]) -> AppResult<usize> {
        // An image takes longer to appear than a keystroke — the agent has to
        // take it from the clipboard before it can draw it — and how much
        // longer is not ours to know, so this waits as patiently as the walk.
        let deadline = Instant::now() + PANE_SETTLE * PANE_SETTLE_ATTEMPTS as u32;
        while Instant::now() < deadline {
            if let Some(marker) = self
                .image_markers()?
                .into_iter()
                .find(|marker| !before.contains(marker))
            {
                return Ok(marker);
            }
            thread::sleep(PANE_SETTLE);
        }
        Err(AppError::new(
            "image paste",
            "native agent did not confirm the image attachment",
        ))
    }
}

/// A character typed into the composer only to see where the cursor is, and
/// deleted again immediately.
const CURSOR_PROBE: &str = "~";

/// A pane redraws after it is typed into, not while, and how long that takes is
/// not ours to know: the multiplexer has its own queue, and the agent redraws
/// when it gets round to it. Measured directly it takes tens of milliseconds;
/// through a busy overlay it has been seen to take longer than half a second,
/// which is what made the walk give up on a probe that did arrive.
///
/// So it waits, and it can afford to: removal is something a person asked for,
/// nothing is pressed until the probe is seen, and waiting costs only time.
const PANE_SETTLE: Duration = Duration::from_millis(100);
const PANE_SETTLE_ATTEMPTS: usize = 30;

/// Removes one image from the native composer, without ever guessing.
///
/// Measured on a live pane: one backspace removes a whole marker, and the
/// cursor steps over a marker as a single unit. What is not certain is how many
/// steps separate the end of the composer from a given marker — the trailing
/// space makes the arithmetic ambiguous, and a step too far would delete the
/// wrong picture.
///
/// So no arithmetic is trusted. The cursor walks back one step at a time and
/// each position is confirmed by typing a character and reading where it landed;
/// only when it sits directly behind the wanted marker is anything deleted, and
/// the result is read back before the removal is reported. The only thing ever
/// written into the composer is that probe, which is erased again at once.
pub fn remove_native_attachment(
    kind: AgentKind,
    marker: usize,
    mut read: impl FnMut() -> AppResult<String>,
    mut press: impl FnMut(&[&str], Option<&str>) -> AppResult<()>,
) -> AppResult<bool> {
    let markers = composer_markers(kind, &read()?);
    let Some(position) = markers.iter().position(|held| *held == marker) else {
        // The pane holds no such image. Only when it holds none at all is that
        // the picture already being gone; otherwise the overlay is naming an
        // image by a number that has gone stale, and reporting success would
        // drop a chip while the picture stays.
        return Ok(markers.is_empty());
    };
    let trailing = markers.len() - position - 1;
    let needle = format!("[Image #{marker}]{CURSOR_PROBE}");

    press(&["ctrl+e"], None)?;
    for _ in 0..=(2 * trailing + 2) {
        press(&[], Some(CURSOR_PROBE))?;
        // Nothing is pressed until the probe is seen. A backspace sent while
        // the probe is not there deletes whatever is there instead, and what is
        // there is somebody's picture.
        let Some(probed) = settle(kind, &mut read, |content| content.contains(CURSOR_PROBE))?
        else {
            return Err(walk_failure(
                &format!(
                    "the composer never showed the probe within {}ms",
                    (PANE_SETTLE * PANE_SETTLE_ATTEMPTS as u32).as_millis()
                ),
                kind,
                &mut read,
            ));
        };
        let placed = probed.contains(&needle);
        press(&["backspace"], None)?;
        let Some(cleaned) = settle(kind, &mut read, |content| !content.contains(CURSOR_PROBE))?
        else {
            return Err(walk_failure(
                "the probe would not come back out of the composer",
                kind,
                &mut read,
            ));
        };
        if placed {
            press(&["backspace"], None)?;
            let gone = format!("[Image #{marker}]");
            if settle(kind, &mut read, |content| !content.contains(&gone))?.is_none() {
                return Err(walk_failure("the image did not go", kind, &mut read));
            }
            let left = composer_markers(kind, &read()?);
            return Ok(!left.contains(&marker) && left.len() + 1 == markers.len());
        }
        let _ = cleaned;
        press(&["left"], None)?;
    }
    Err(walk_failure(
        "the cursor never reached the image",
        kind,
        &mut read,
    ))
}

/// An error that carries what the composer actually looked like when the walk
/// gave up, so the next report says what happened rather than that it did.
fn walk_failure(
    reason: &str,
    kind: AgentKind,
    read: &mut impl FnMut() -> AppResult<String>,
) -> AppError {
    let seen = read()
        .ok()
        .and_then(|ansi| native_composer_content(kind, &sanitize_ansi(&ansi)))
        .unwrap_or_else(|| "<unreadable>".to_owned());
    AppError::new(
        "remove image",
        format!("{reason}; composer showed {seen:?}"),
    )
}

/// Reads the pane until it shows what was just typed into it.
///
/// Returns the settled content, or `None` if the pane never came to show it —
/// which the caller treats as "not where I wanted", never as "go ahead".
fn settle(
    kind: AgentKind,
    read: &mut impl FnMut() -> AppResult<String>,
    is_settled: impl Fn(&str) -> bool,
) -> AppResult<Option<String>> {
    for _ in 0..PANE_SETTLE_ATTEMPTS {
        let content = native_composer_content(kind, &sanitize_ansi(&read()?));
        if content.as_deref().is_some_and(&is_settled) {
            return Ok(content);
        }
        thread::sleep(PANE_SETTLE);
    }
    Ok(None)
}

/// Every image the composer is showing, wherever it sits.
///
/// This reads the markers where they are rather than insisting they come before
/// the text: a composer caught mid-edit is a normal thing to look at, and a
/// count that quietly returns none for it made a freshly pasted image look like
/// it had never arrived.
fn composer_markers(kind: AgentKind, ansi: &str) -> Vec<usize> {
    native_composer_content(kind, &sanitize_ansi(ansi))
        .map(|content| scan_markers(&content))
        .unwrap_or_default()
}

fn scan_markers(content: &str) -> Vec<usize> {
    let mut markers = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("[Image #") {
        let after = &rest[start + "[Image #".len()..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with(']') {
            if let Ok(number) = after[..digits].parse() {
                markers.push(number);
            }
        }
        rest = &after[digits..];
    }
    markers
}

#[cfg(test)]
mod tests {
    use super::{CURSOR_PROBE, remove_native_attachment};
    use crate::agent::AgentKind;
    use crate::{AppError, AppResult};
    use std::cell::RefCell;

    /// A native composer as the measurements described one: a marker is a
    /// single unit both to step over and to delete, and a trailing space sits
    /// after the last one.
    struct Composer {
        units: Vec<String>,
        cursor: usize,
        ignores_left: bool,
    }

    impl Composer {
        fn new(units: &[&str]) -> Self {
            Self {
                units: units.iter().map(|unit| (*unit).to_owned()).collect(),
                cursor: units.len(),
                ignores_left: false,
            }
        }

        fn surface(&self) -> String {
            format!(
                "• answer\n────────\n› {}\ngpt-5.6-sol xhigh · /repo · weekly 75% left",
                self.units.concat()
            )
        }

        fn markers(&self) -> Vec<String> {
            self.units
                .iter()
                .filter(|unit| unit.starts_with("[Image #"))
                .cloned()
                .collect()
        }

        fn press(&mut self, keys: &[&str], text: Option<&str>) {
            for key in keys {
                match *key {
                    "ctrl+e" => self.cursor = self.units.len(),
                    "left" => {
                        if !self.ignores_left {
                            self.cursor = self.cursor.saturating_sub(1);
                        }
                    }
                    "backspace" => {
                        if self.cursor > 0 {
                            self.cursor -= 1;
                            self.units.remove(self.cursor);
                        }
                    }
                    other => panic!("unmodelled key {other}"),
                }
            }
            for character in text.unwrap_or_default().chars() {
                self.units.insert(self.cursor, character.to_string());
                self.cursor += 1;
            }
        }
    }

    fn three_images() -> Composer {
        Composer::new(&["[Image #5]", " ", "[Image #6]", " ", "[Image #7]", " "])
    }

    fn remove(composer: &RefCell<Composer>, marker: usize) -> AppResult<bool> {
        remove_native_attachment(
            AgentKind::Codex,
            marker,
            || Ok(composer.borrow().surface()),
            |keys, text| {
                composer.borrow_mut().press(keys, text);
                Ok(())
            },
        )
    }

    /// A composer caught mid-edit still has to be counted: an image typed after
    /// text, or a probe sitting beside one, is a normal thing to look at, and a
    /// count that returned none for it made a pasted image look like it had
    /// never arrived.
    #[test]
    fn images_are_counted_wherever_they_sit() {
        assert_eq!(super::scan_markers("[Image #5] [Image #6] "), [5, 6]);
        assert_eq!(super::scan_markers("describe [Image #5] it"), [5]);
        assert_eq!(super::scan_markers("[Image #5]~ [Image #6]"), [5, 6]);
        assert_eq!(super::scan_markers("no images here"), Vec::<usize>::new());
        assert_eq!(super::scan_markers("[Image #] [Image #7]"), [7]);
    }

    #[test]
    fn the_last_image_is_removed_and_the_others_are_left_alone() {
        let composer = RefCell::new(three_images());

        assert!(remove(&composer, 7).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #5]", "[Image #6]"]);
    }

    #[test]
    fn an_image_in_the_middle_of_the_line_is_reached_and_removed() {
        let composer = RefCell::new(three_images());

        assert!(remove(&composer, 6).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #5]", "[Image #7]"]);
    }

    #[test]
    fn the_first_image_is_reached_and_removed() {
        let composer = RefCell::new(three_images());

        assert!(remove(&composer, 5).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #6]", "[Image #7]"]);
    }

    /// Nothing is deleted from a position that could not be confirmed. A
    /// composer that will not move its cursor must cost the caller a refusal,
    /// never someone else's picture.
    #[test]
    fn a_cursor_that_will_not_move_costs_a_refusal_not_a_picture() {
        let composer = RefCell::new(three_images());
        composer.borrow_mut().ignores_left = true;

        let refusal = remove(&composer, 5).unwrap_err().to_string();
        assert!(
            refusal.contains("never reached"),
            "the refusal says what happened: {refusal}"
        );
        assert_eq!(
            composer.borrow().markers(),
            ["[Image #5]", "[Image #6]", "[Image #7]"],
            "every image survives a walk that never found its place"
        );
    }

    /// A number the pane does not know is not proof the picture is gone — it is
    /// proof the overlay is naming it wrongly. Reporting success there would
    /// drop a chip while the picture stays, which is the disagreement the whole
    /// guard exists to prevent.
    #[test]
    fn a_number_the_pane_does_not_know_is_refused_while_it_holds_images() {
        let composer = RefCell::new(three_images());

        assert!(!remove(&composer, 99).unwrap());

        assert_eq!(
            composer.borrow().markers(),
            ["[Image #5]", "[Image #6]", "[Image #7]"],
        );
    }

    #[test]
    fn an_image_is_reported_removed_when_the_composer_holds_none() {
        let composer = RefCell::new(Composer::new(&["describe it"]));

        assert!(remove(&composer, 5).unwrap());
        assert!(composer.borrow().markers().is_empty());
    }

    #[test]
    fn the_probe_never_survives_the_walk() {
        let composer = RefCell::new(three_images());

        remove(&composer, 5).unwrap();

        assert!(
            !composer.borrow().units.concat().contains(CURSOR_PROBE),
            "the character used to find the cursor is always erased again"
        );
    }

    #[test]
    fn a_read_failure_stops_the_walk_before_anything_is_pressed() {
        let composer = RefCell::new(three_images());
        let result = remove_native_attachment(
            AgentKind::Codex,
            5,
            || Err(AppError::new("test", "pane unavailable")),
            |_, _| panic!("nothing may be pressed without reading the pane first"),
        );

        assert!(result.is_err());
        assert_eq!(composer.borrow().markers().len(), 3);
    }
}
