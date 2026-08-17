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
        // An image takes longer to appear than a keystroke: the agent has to
        // take it out of the clipboard and put it somewhere before it can draw
        // it, and a screenshot is not small. How long that is is not ours to
        // know, so this waits well past what has been seen rather than guessing
        // a limit and calling a slow paste a failed one.
        let started = Instant::now();
        let mut seen = Vec::new();
        while started.elapsed() < IMAGE_PASTE_WINDOW {
            seen = self.image_markers()?;
            if let Some(marker) = seen.iter().find(|marker| !before.contains(marker)) {
                return Ok(*marker);
            }
            thread::sleep(PANE_SETTLE);
        }
        Err(AppError::new(
            "image paste",
            format!(
                "native agent did not confirm the image attachment in {}s; \
                 composer held {before:?} before and {seen:?} after",
                IMAGE_PASTE_WINDOW.as_secs()
            ),
        ))
    }
}

/// A pause between keys sent one after another.
///
/// Each request travels on a connection of its own, so two sent back to back
/// are not promised to arrive in that order. Keys are cheap; this costs a few
/// milliseconds and buys the order they were written in.
const KEY_SPACING: Duration = Duration::from_millis(50);

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

/// How long an agent may take to attach a pasted image before the overlay calls
/// it a failure. Generous on purpose: a paste that is merely slow must not be
/// reported as one that did not happen.
const IMAGE_PASTE_WINDOW: Duration = Duration::from_secs(10);

const MARKER_OPEN: &str = "[Image #";

/// Characters to mark the composer with, tried in turn: whichever the composer
/// does not already hold is the one the mark cannot be confused with.
const PROBES: [char; 4] = ['¤', '¦', '‡', '¬'];

/// How many times the walk may correct itself before giving up. Each round
/// measures where the caret actually is, so a round that lands wrong makes the
/// next one exact; more than a few means the composer is not answering.
const PLACE_ATTEMPTS: usize = 4;

/// Removes one image from the native composer, without ever guessing.
///
/// Measured on a live pane: an image marker is a single unit — one arrow key
/// crosses it and one backspace takes it whole — and the composer gives up any
/// image the caret stands behind, not only the last one. What cannot be read
/// back is the caret: the pane reports what it holds and never where the caret
/// is.
///
/// So the caret is established rather than assumed. A character the user could
/// not have typed is written into the composer and the pane is read back: where
/// that mark appears is where the typing landed, which is where the caret was.
/// The mark is taken out again at once. Nothing is deleted until a mark has
/// been seen sitting directly behind the wanted image.
///
/// Two measured quirks are honoured. A terminal pads every row with blanks, so
/// a space held at the end of the composer cannot be told apart from the empty
/// rest of the row — a mark typed at the end makes those spaces visible, and
/// only then does the counting mean anything. And a mark typed directly in
/// front of an image leaves the caret past that image, so the backspace meant
/// for the mark needs a step back first; without it, it takes the picture.
pub fn remove_native_attachment(
    kind: AgentKind,
    marker: usize,
    mut read: impl FnMut() -> AppResult<String>,
    mut send: impl FnMut(&[&str], Option<&str>) -> AppResult<()>,
) -> AppResult<bool> {
    let mut press = move |keys: &[&str], text: Option<&str>| -> AppResult<()> {
        send(keys, text)?;
        thread::sleep(KEY_SPACING);
        Ok(())
    };
    let markers = composer_markers(kind, &read()?);
    if !markers.contains(&marker) {
        // The pane holds no such image. Only when it holds none at all is that
        // the picture already being gone; otherwise the overlay is naming an
        // image by a number that has gone stale, and reporting success would
        // drop a chip while the picture stays.
        return Ok(markers.is_empty());
    }
    remove_by_walking(kind, marker, &markers, &mut read, &mut press)
}

/// Brings the caret to the image and takes it, whichever image it is.
///
/// The last one used to be a case of its own: the end of the composer is one
/// keystroke away, so the presses were simply counted out — one for the space
/// beside the picture and one for the picture. But every removal leaves that
/// space behind, so a composer that has lost a few images ends in several of
/// them, and the counted presses ate spaces and then reported that the image
/// had not gone while it was still sitting there. Counting is what this walk
/// exists to avoid, so the last image is no longer an exception to it.
fn remove_by_walking(
    kind: AgentKind,
    marker: usize,
    before: &[usize],
    read: &mut impl FnMut() -> AppResult<String>,
    press: &mut impl FnMut(&[&str], Option<&str>) -> AppResult<()>,
) -> AppResult<bool> {
    let start = composer_content(kind, read)?;
    let probe = probe_character(&start)?;

    // The end of the composer is the one place reached without counting, and a
    // mark typed there shows what the padded row was hiding.
    press(&["ctrl+e"], None)?;
    let revealed = place_probe(kind, probe, &start, read, press)?;
    let end = revealed.len() - 1;
    if probe_position(&revealed, probe) != Some(end) {
        return Err(walk_failure("the end of the composer moved", kind, read));
    }
    let mut held = revealed[..end].to_vec();
    withdraw_probe(kind, probe, &revealed, end, &render(&held), read, press)?;
    let mut caret = held.len();

    for _ in 0..PLACE_ATTEMPTS {
        let Some(image) = held.iter().position(|unit| *unit == Unit::Marker(marker)) else {
            return Err(walk_failure("the image left the composer", kind, read));
        };
        let wanted = image + 1;
        step(caret, wanted, press)?;
        let probed = place_probe(kind, probe, &render(&held), read, press)?;
        let Some(at) = probe_position(&probed, probe) else {
            return Err(walk_failure("the mark could not be found", kind, read));
        };
        // Behind the image and nothing else: the mark itself says so, which is
        // the whole reason it was typed.
        let behind_the_image = at == wanted && probed.get(image) == Some(&Unit::Marker(marker));
        withdraw_probe(kind, probe, &probed, at, &render(&held), read, press)?;
        // Taking the mark out leaves the caret where the mark stood, so its
        // place is now the caret's place — measured, not assumed.
        caret = at;
        held = without(&probed, at);
        if behind_the_image {
            press(&["backspace"], None)?;
            let gone = format!("[Image #{marker}]");
            if settle(kind, read, |content| !content.contains(&gone))?.is_none() {
                return Err(walk_failure("the image did not go", kind, read));
            }
            return confirm_removal(kind, marker, before, read);
        }
    }
    Err(walk_failure(
        "the caret could not be brought to the image",
        kind,
        read,
    ))
}

/// Reports the removal only if exactly the wanted picture left.
fn confirm_removal(
    kind: AgentKind,
    marker: usize,
    before: &[usize],
    read: &mut impl FnMut() -> AppResult<String>,
) -> AppResult<bool> {
    let left = composer_markers(kind, &read()?);
    Ok(!left.contains(&marker) && left.len() + 1 == before.len())
}

/// Types the mark and reads back where it landed.
///
/// The mark must be the only thing that changed: if the composer holds anything
/// else new, something other than this walk is writing into it, and nothing may
/// be deleted on the strength of a picture that has already gone stale.
fn place_probe(
    kind: AgentKind,
    probe: char,
    expected: &str,
    read: &mut impl FnMut() -> AppResult<String>,
    press: &mut impl FnMut(&[&str], Option<&str>) -> AppResult<()>,
) -> AppResult<Vec<Unit>> {
    press(&[], Some(&probe.to_string()))?;
    let Some(content) = settle(kind, read, |content| content.contains(probe))? else {
        return Err(walk_failure(
            "the composer never showed the mark",
            kind,
            read,
        ));
    };
    let placed = units(&content);
    let Some(at) = probe_position(&placed, probe) else {
        return Err(walk_failure("the mark could not be found", kind, read));
    };
    if render(&without(&placed, at)).trim_end() != expected.trim_end() {
        return Err(walk_failure(
            "the composer changed while the mark was placed",
            kind,
            read,
        ));
    }
    Ok(placed)
}

/// Takes the mark back out, leaving the caret where the mark stood.
fn withdraw_probe(
    kind: AgentKind,
    probe: char,
    placed: &[Unit],
    at: usize,
    expected: &str,
    read: &mut impl FnMut() -> AppResult<String>,
    press: &mut impl FnMut(&[&str], Option<&str>) -> AppResult<()>,
) -> AppResult<()> {
    // A mark typed directly in front of an image leaves the caret past that
    // image. Stepping back is what keeps this backspace on the mark instead of
    // on the picture.
    if matches!(placed.get(at + 1), Some(Unit::Marker(_))) {
        press(&["left"], None)?;
    }
    press(&["backspace"], None)?;
    let Some(content) = settle(kind, read, |content| !content.contains(probe))? else {
        return Err(walk_failure("the mark stayed in the composer", kind, read));
    };
    if content.trim_end() != expected.trim_end() {
        return Err(walk_failure(
            "taking the mark out changed the composer",
            kind,
            read,
        ));
    }
    Ok(())
}

fn step(
    from: usize,
    to: usize,
    press: &mut impl FnMut(&[&str], Option<&str>) -> AppResult<()>,
) -> AppResult<()> {
    let (key, count) = if from > to {
        ("left", from - to)
    } else {
        ("right", to - from)
    };
    for _ in 0..count {
        press(&[key], None)?;
    }
    Ok(())
}

fn composer_content(
    kind: AgentKind,
    read: &mut impl FnMut() -> AppResult<String>,
) -> AppResult<String> {
    native_composer_content(kind, &sanitize_ansi(&read()?))
        .ok_or_else(|| AppError::new("remove image", "the native composer could not be read"))
}

/// A character to mark the composer with. It has to be one the user could not
/// have written there, or the mark could be mistaken for their own text.
fn probe_character(content: &str) -> AppResult<char> {
    PROBES
        .iter()
        .copied()
        .find(|probe| !content.contains(*probe))
        .ok_or_else(|| {
            AppError::new(
                "remove image",
                "the composer already holds every mark this could use",
            )
        })
}

fn probe_position(units: &[Unit], probe: char) -> Option<usize> {
    units
        .iter()
        .position(|unit| *unit == Unit::Character(probe))
}

fn without(units: &[Unit], at: usize) -> Vec<Unit> {
    let mut rest = units.to_vec();
    rest.remove(at);
    rest
}

/// One step of the composer: an image marker is a single unit, the same as a
/// single character.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Unit {
    Marker(usize),
    Character(char),
}

fn units(content: &str) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut rest = content;
    while let Some(character) = rest.chars().next() {
        match marker_at(rest) {
            Some((number, length)) => {
                units.push(Unit::Marker(number));
                rest = &rest[length..];
            }
            None => {
                units.push(Unit::Character(character));
                rest = &rest[character.len_utf8()..];
            }
        }
    }
    units
}

fn marker_at(content: &str) -> Option<(usize, usize)> {
    let after = content.strip_prefix(MARKER_OPEN)?;
    let digits = after.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || !after[digits..].starts_with(']') {
        return None;
    }
    Some((
        after[..digits].parse().ok()?,
        MARKER_OPEN.len() + digits + 1,
    ))
}

fn render(units: &[Unit]) -> String {
    let mut content = String::new();
    for unit in units {
        match unit {
            Unit::Marker(number) => {
                content.push_str(MARKER_OPEN);
                content.push_str(&number.to_string());
                content.push(']');
            }
            Unit::Character(character) => content.push(*character),
        }
    }
    content
}

/// An error that carries what the composer actually looked like when the
/// removal gave up, so the next report says what happened rather than that it
/// did.
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
    units(content)
        .into_iter()
        .filter_map(|unit| match unit {
            Unit::Marker(number) => Some(number),
            Unit::Character(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::remove_native_attachment;
    use crate::agent::AgentKind;
    use crate::{AppError, AppResult};
    use std::cell::RefCell;

    /// A native composer as the measurements described one: a marker is a
    /// single unit both to step over and to delete, and a trailing space sits
    /// after the last one.
    struct Composer {
        units: Vec<String>,
        cursor: usize,
    }

    impl Composer {
        fn new(units: &[&str]) -> Self {
            Self {
                units: units.iter().map(|unit| (*unit).to_owned()).collect(),
                cursor: units.len(),
            }
        }

        /// What the pane shows — which is not quite what the composer holds. A
        /// terminal pads every row with blanks, so trailing spaces come back
        /// indistinguishable from the empty rest of the row, exactly as they do
        /// from a live pane.
        fn surface(&self) -> String {
            format!(
                "• answer\n────────\n› {}\ngpt-5.6-sol xhigh · /repo · weekly 75% left",
                self.units.concat().trim_end_matches(' ')
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
                    "ctrl+a" => self.cursor = 0,
                    "left" => self.cursor = self.cursor.saturating_sub(1),
                    "right" => self.cursor = (self.cursor + 1).min(self.units.len()),
                    "backspace" => {
                        if self.cursor > 0 {
                            self.cursor -= 1;
                            self.units.remove(self.cursor);
                        }
                    }
                    other => panic!("unmodelled key {other}"),
                }
            }
            let Some(text) = text else {
                return;
            };
            for character in text.chars() {
                self.units.insert(self.cursor, character.to_string());
                self.cursor += 1;
            }
            // Measured on a live pane, three times over: typing directly in
            // front of an image leaves the caret past that image, so a
            // backspace meant to take the typing back takes the picture
            // instead.
            if self.is_image(self.cursor) {
                self.cursor += 1;
            }
        }

        fn is_image(&self, index: usize) -> bool {
            self.units
                .get(index)
                .is_some_and(|unit| unit.starts_with("[Image #"))
        }
    }

    fn three_images() -> Composer {
        Composer::new(&["[Image #5]", " ", "[Image #6]", " ", "[Image #7]"])
    }

    /// What an earlier removal leaves behind: the space that separated the
    /// image from the one before it still sits at the end.
    fn two_images_and_a_space() -> Composer {
        Composer::new(&["[Image #5]", " ", "[Image #6]", " "])
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

    #[test]
    fn the_newest_image_is_removed_and_the_others_are_left_alone() {
        let composer = RefCell::new(three_images());

        assert!(remove(&composer, 7).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #5]", "[Image #6]"]);
    }

    /// A space left at the end by an earlier removal must not be mistaken for
    /// the image.
    #[test]
    fn a_space_left_at_the_end_does_not_stop_the_removal() {
        let composer = RefCell::new(two_images_and_a_space());

        assert!(remove(&composer, 6).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #5]"]);
    }

    /// Every removal leaves behind the space that sat beside the picture, so
    /// the end of a composer that has lost a few images is several spaces the
    /// pane cannot show. Counting presses instead of asking took those spaces
    /// for the picture and then said the picture had not gone — while it was
    /// still there for anyone to see.
    #[test]
    fn spaces_left_by_earlier_removals_do_not_stop_the_last_one() {
        let composer = RefCell::new(Composer::new(&[
            "[Image #5]",
            " ",
            "[Image #6]",
            " ",
            " ",
            " ",
        ]));

        assert!(remove(&composer, 6).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #5]"]);
    }

    #[test]
    fn an_earlier_image_is_removed_and_the_others_are_left_alone() {
        for (marker, left) in [
            (5, vec!["[Image #6]", "[Image #7]"]),
            (6, vec!["[Image #5]", "[Image #7]"]),
        ] {
            let composer = RefCell::new(three_images());

            assert!(remove(&composer, marker).unwrap());
            assert_eq!(composer.borrow().markers(), left);
        }
    }

    /// A picture with text on both sides is the one the caret has to be steered
    /// into, so it is the case that proves the steering.
    #[test]
    fn an_image_between_text_is_removed_without_touching_the_text() {
        let composer = RefCell::new(Composer::new(&[
            "l",
            "o",
            "o",
            "k",
            " ",
            "[Image #5]",
            " ",
            "[Image #6]",
            " ",
            "h",
            "e",
            "r",
            "e",
        ]));

        assert!(remove(&composer, 5).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #6]"]);
        assert_eq!(composer.borrow().units.concat(), "look  [Image #6] here");
    }

    /// The composer holds a space after the last image that the pane cannot
    /// show, so counting back from the end without asking would land one step
    /// short — on the picture instead of behind it.
    #[test]
    fn a_space_the_pane_cannot_show_does_not_misplace_the_caret() {
        let composer = RefCell::new(Composer::new(&[
            "[Image #5]",
            " ",
            "[Image #6]",
            " ",
            "[Image #7]",
            " ",
        ]));

        assert!(remove(&composer, 5).unwrap());
        assert_eq!(composer.borrow().markers(), ["[Image #6]", "[Image #7]"]);
    }

    /// Nothing is deleted on the strength of a picture that has gone stale: if
    /// the composer changes under the walk, the walk stops.
    #[test]
    fn a_composer_that_changes_under_the_walk_stops_it() {
        let composer = RefCell::new(three_images());
        let mut reads = 0;

        let outcome = remove_native_attachment(
            AgentKind::Codex,
            5,
            || {
                reads += 1;
                if reads > 3 {
                    composer.borrow_mut().units.push("typed".to_owned());
                }
                Ok(composer.borrow().surface())
            },
            |keys, text| {
                composer.borrow_mut().press(keys, text);
                Ok(())
            },
        );

        assert!(outcome.is_err(), "the walk stops rather than deleting");
        assert_eq!(
            composer.borrow().markers(),
            ["[Image #5]", "[Image #6]", "[Image #7]"],
            "and every picture is still there"
        );
    }

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
    fn a_read_failure_stops_before_anything_is_pressed() {
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

    #[test]
    fn images_are_counted_wherever_they_sit() {
        assert_eq!(super::scan_markers("[Image #5] [Image #6] "), [5, 6]);
        assert_eq!(super::scan_markers("describe [Image #5] it"), [5]);
        assert_eq!(super::scan_markers("[Image #5]x [Image #6]"), [5, 6]);
        assert_eq!(super::scan_markers("no images here"), Vec::<usize>::new());
        assert_eq!(super::scan_markers("[Image #] [Image #7]"), [7]);
    }
}
