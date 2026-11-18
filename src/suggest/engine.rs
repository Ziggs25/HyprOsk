use crate::suggest::dictionary::Dictionary;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct PreselectionSnapshot {
    surrounding: String,
    cursor: usize,
    anchor: usize,
}

#[derive(Debug, Clone)]
struct PendingSelectionEcho {
    selected_word: String,
    previous_history: Vec<String>,
    previous_current_word: String,
    preselection_snapshot: Option<PreselectionSnapshot>,
    started_at: Instant,
}

#[derive(Debug, Clone)]
pub struct SuggestEngine {
    pub dictionary: Dictionary,
    pub current_word: String,
    pub word_history: Vec<String>,
    pub candidates: Vec<String>,
    pub is_next_word_mode: bool,
    pending_selection_echo: Option<PendingSelectionEcho>,
    next_preselection_snapshot: Option<PreselectionSnapshot>,
    last_local_word_input_at: Option<Instant>,
}

impl Default for SuggestEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SuggestEngine {
    pub fn new() -> Self {
        Self {
            dictionary: Dictionary::new(),
            current_word: String::new(),
            word_history: Vec::new(),
            candidates: Vec::new(),
            is_next_word_mode: false,
            pending_selection_echo: None,
            next_preselection_snapshot: None,
            last_local_word_input_at: None,
        }
    }

    /// Creates a SuggestEngine with empty user dictionary (isolated from disk).
    pub fn new_empty() -> Self {
        Self {
            dictionary: Dictionary::new_empty(),
            current_word: String::new(),
            word_history: Vec::new(),
            candidates: Vec::new(),
            is_next_word_mode: false,
            pending_selection_echo: None,
            next_preselection_snapshot: None,
            last_local_word_input_at: None,
        }
    }

    pub fn push_char(&mut self, ch: char) {
        if ch.is_alphabetic() || ch == '\'' {
            self.last_local_word_input_at = Some(Instant::now());
            if self.is_next_word_mode {
                self.is_next_word_mode = false;
                self.current_word.clear();
            }
            self.current_word.push(ch);
            self.recalculate();
        } else if ch == ' ' {
            self.last_local_word_input_at = None;
            self.on_space();
        } else if ch == '.' || ch == '?' || ch == '!' {
            self.last_local_word_input_at = None;
            if !self.current_word.is_empty() {
                let typed = std::mem::take(&mut self.current_word);
                self.word_history.push(typed);
            }
            self.word_history.clear();
            self.clear();
        } else {
            self.last_local_word_input_at = None;
            self.on_space();
        }
    }

    pub fn pop_char(&mut self) {
        self.last_local_word_input_at = None;
        if self.is_next_word_mode {
            // The cursor was right after a space following the last word.
            // This backspace deleted that trailing space from the text box.
            self.is_next_word_mode = false;
            if let Some(last) = self.word_history.pop() {
                // HeliBoard-style backspace undo: Revert transient unconfirmed word/bigram learning
                self.dictionary.revert_last_user_word(&last);
                self.current_word = last;
                self.recalculate();
            } else {
                self.candidates.clear();
            }
            return;
        }

        if !self.current_word.is_empty() {
            self.current_word.pop();
            if self.current_word.is_empty() {
                // The current word was completely backspaced down to 0 chars.
                // Predict the next words following the word before the space!
                if let Some(prev) = self.word_history.last().cloned() {
                    self.is_next_word_mode = true;
                    self.predict_next(&prev);
                } else {
                    self.candidates.clear();
                }
            } else {
                self.recalculate();
            }
        } else if let Some(last) = self.word_history.pop() {
            // User is backspacing across the space before the previous word!
            self.is_next_word_mode = false;
            self.current_word = last;
            self.recalculate();
        } else {
            self.candidates.clear();
        }
    }

    pub fn sync_with_text(&mut self, surrounding: &str, cursor_bytes: usize) {
        self.sync_with_snapshot(surrounding, cursor_bytes, cursor_bytes, false);
    }

    pub fn sync_with_external_text(&mut self, surrounding: &str, cursor_bytes: usize) {
        self.sync_with_snapshot(surrounding, cursor_bytes, cursor_bytes, true);
    }

    pub fn sync_with_snapshot(
        &mut self,
        surrounding: &str,
        cursor_bytes: usize,
        anchor_bytes: usize,
        external: bool,
    ) {
        self.sync_with_text_inner(surrounding, cursor_bytes, anchor_bytes, external);
    }

    pub fn is_recent_inline_completion_echo(
        &self,
        surrounding: &str,
        cursor_bytes: usize,
        anchor_bytes: usize,
    ) -> bool {
        let Some(prefix) = inline_completion_prefix(surrounding, cursor_bytes, anchor_bytes) else {
            return false;
        };
        self.is_recent_strict_word_prefix(prefix)
    }

    pub fn is_recent_atspi_inline_completion_echo(
        &self,
        surrounding: &str,
        cursor_bytes: usize,
    ) -> bool {
        if cursor_bytes >= surrounding.len() || !surrounding.is_char_boundary(cursor_bytes) {
            return false;
        }
        let prefix = surrounding[..cursor_bytes]
            .rsplit(|ch: char| !ch.is_alphabetic() && ch != '\'')
            .next()
            .unwrap_or("");
        self.is_recent_strict_word_prefix(prefix)
    }

    fn is_recent_strict_word_prefix(&self, prefix: &str) -> bool {
        let prefix_chars = prefix.chars().count();
        let tracked_prefix = self.current_word.chars().take(prefix_chars).collect::<String>();
        !self.current_word.is_empty()
            && !prefix.is_empty()
            && prefix_chars < self.current_word.chars().count()
            && tracked_prefix.eq_ignore_ascii_case(prefix)
            && self
                .last_local_word_input_at
                .is_some_and(|at| at.elapsed() <= Duration::from_millis(500))
    }

    fn sync_with_text_inner(
        &mut self,
        surrounding: &str,
        cursor_bytes: usize,
        anchor_bytes: usize,
        external: bool,
    ) {
        let caret_bytes = cursor_bytes.min(anchor_bytes);
        let valid_len = surrounding.floor_char_boundary(surrounding.len().min(caret_bytes));
        let before = &surrounding[..valid_len];
        let words: Vec<String> = before
            .split(|c: char| !c.is_alphabetic() && c != '\'')
            .filter(|w| !w.is_empty())
            .map(|w| w.to_string())
            .collect();

        if !external
            && self.is_recent_inline_completion_echo(surrounding, cursor_bytes, anchor_bytes)
        {
            tracing::debug!(
                surrounding_bytes = surrounding.len(),
                cursor_bytes,
                anchor_bytes,
                current_word_bytes = self.current_word.len(),
                "ignoring stale inline-completion snapshot for suggestions"
            );
            return;
        }

        if self.should_ignore_pending_selection_echo(
            surrounding,
            cursor_bytes,
            anchor_bytes,
            valid_len,
            before,
            &words,
        ) {
            return;
        }

        if before.is_empty() {
            // Lagging echo from before the word started (e.g. empty/cleared
            // event arriving after typing began): preserve the active word.
            if !external && !self.current_word.is_empty() {
                return;
            }
            self.clear();
            return;
        }

        if before.ends_with(' ') {
            if !external && !self.current_word.is_empty() {
                return;
            }
            self.word_history = words;
            self.current_word.clear();
            if let Some(last_word) = self.word_history.last().cloned() {
                self.is_next_word_mode = true;
                self.predict_next(&last_word);
            } else {
                self.clear();
            }
        } else if before
            .chars()
            .last()
            .is_some_and(|c| !c.is_alphabetic() && c != '\'')
        {
            // Cursor is directly after punctuation or symbol (e.g. '.', '/', ':')
            self.word_history = words;
            self.current_word.clear();
            self.clear();
        } else if let Some(last_word) = words.last().cloned() {
            // Stale post-commit echo: if next-word prediction is active and the
            // surrounding snapshot reports an unspaced prefix of the word just
            // committed, ignore it so the optimistic selection state is not reverted.
            if !external
                && self.is_next_word_mode
                && self
                    .word_history
                    .last()
                    .map_or(false, |last| last.starts_with(&last_word))
            {
                return;
            }
            // Lagging prefix echo: Chromium may deliver stale surrounding text
            // (e.g. 'b' arriving after 'r' was typed for 'br'). If the
            // surrounding word is a strict prefix of the actively typed word,
            // it is stale — return without clobbering current_word.
            if !external
                && !self.current_word.is_empty()
                && last_word.len() < self.current_word.len()
                && self.current_word.starts_with(&last_word)
            {
                return;
            }
            self.word_history = words[..words.len() - 1].to_vec();
            self.current_word = last_word;
            self.is_next_word_mode = false;
            self.recalculate();
        } else {
            self.clear();
        }
    }

    fn should_ignore_pending_selection_echo(
        &mut self,
        surrounding: &str,
        cursor_bytes: usize,
        anchor_bytes: usize,
        before_cursor_bytes: usize,
        before: &str,
        words: &[String],
    ) -> bool {
        let Some(pending) = self.pending_selection_echo.clone() else {
            return false;
        };

        if pending.started_at.elapsed() > Duration::from_millis(1800) {
            self.pending_selection_echo = None;
            return false;
        }

        if pending
            .preselection_snapshot
            .as_ref()
            .is_some_and(|snapshot| {
                snapshot.surrounding == surrounding
                    && snapshot.cursor == cursor_bytes
                    && snapshot.anchor == anchor_bytes
            })
        {
            return true;
        }

        if selected_text_covers_cursor(
            surrounding,
            before_cursor_bytes,
            &pending.selected_word,
        ) {
            self.pending_selection_echo = None;
            return false;
        }

        if matches_preselection_context(&pending, before, words) {
            return true;
        }

        self.pending_selection_echo = None;
        false
    }

    pub fn on_space(&mut self) {
        self.last_local_word_input_at = None;
        if !self.current_word.is_empty() {
            let typed = std::mem::take(&mut self.current_word);
            // Record unigram in user dictionary (transient on first type)
            self.dictionary.record_user_word(&typed, false);
            if let Some(prev) = self.word_history.last() {
                self.dictionary.record_user_bigram(prev, &typed);
            }
            self.word_history.push(typed.clone());
            self.is_next_word_mode = true;
            self.predict_next(&typed);
        } else if let Some(last) = self.word_history.last().cloned() {
            self.is_next_word_mode = true;
            self.predict_next(&last);
        } else {
            self.clear();
        }
    }

    pub fn remember_preselection_snapshot(
        &mut self,
        surrounding: Option<&str>,
        cursor: usize,
        anchor: usize,
    ) {
        self.next_preselection_snapshot = surrounding.map(|surrounding| PreselectionSnapshot {
            surrounding: surrounding.to_owned(),
            cursor,
            anchor,
        });
    }

    pub fn on_word_selected(&mut self, chosen_word: &str) {
        let preselection_snapshot = self.next_preselection_snapshot.take();
        self.on_word_committed(chosen_word, true, preselection_snapshot);
    }

    pub fn on_swipe_word_committed(&mut self, chosen_word: &str) {
        self.next_preselection_snapshot = None;
        self.on_word_committed(chosen_word, false, None);
    }

    fn on_word_committed(
        &mut self,
        chosen_word: &str,
        explicit_selection: bool,
        preselection_snapshot: Option<PreselectionSnapshot>,
    ) {
        self.last_local_word_input_at = None;
        let chosen_clean = chosen_word.trim().to_string();
        self.pending_selection_echo = if chosen_clean.is_empty() {
            None
        } else {
            Some(PendingSelectionEcho {
                selected_word: chosen_clean.clone(),
                previous_history: self.word_history.clone(),
                previous_current_word: self.current_word.clone(),
                preselection_snapshot,
                started_at: Instant::now(),
            })
        };
        // A swipe commit is transient; a tapped correction is immediately permanent.
        self.dictionary
            .record_user_word(&chosen_clean, explicit_selection);
        if let Some(prev) = self.word_history.last()
            && !prev.eq_ignore_ascii_case(&chosen_clean)
        {
            self.dictionary.record_user_bigram(prev, &chosen_clean);
        }
        self.word_history.push(chosen_clean.clone());
        self.current_word.clear();
        self.is_next_word_mode = true;
        self.predict_next(&chosen_clean);
    }

    pub fn predict_next(&mut self, prev_word: &str) {
        let predictions = self.dictionary.predict_next_words(prev_word, 3);
        if predictions.is_empty() {
            self.candidates.clear();
            self.is_next_word_mode = false;
            return;
        }

        self.is_next_word_mode = true;
        // predictions are sorted by highest frequency first: [p0, p1, p2]
        // Center slot (idx 1) is the primary recommendation (p0)
        // Left slot (idx 0) is runner-up (p1)
        // Right slot (idx 2) is 3rd candidate (p2)
        if predictions.len() >= 2 {
            let p0 = predictions[0].0.clone();
            let p1 = predictions[1].0.clone();
            let p2 = if predictions.len() >= 3 {
                predictions[2].0.clone()
            } else {
                String::new()
            };
            self.candidates = vec![p1, p0, p2];
        } else {
            self.candidates = predictions.into_iter().map(|(w, _)| w).collect();
        }
    }

    pub fn clear(&mut self) {
        self.current_word.clear();
        self.word_history.clear();
        self.candidates.clear();
        self.is_next_word_mode = false;
        self.pending_selection_echo = None;
        self.next_preselection_snapshot = None;
        self.last_local_word_input_at = None;
    }

    pub fn is_empty(&self) -> bool {
        self.current_word.is_empty() && self.candidates.is_empty()
    }

    pub fn recalculate(&mut self) {
        if self.current_word.is_empty() {
            self.candidates.clear();
            self.is_next_word_mode = false;
            return;
        }

        self.is_next_word_mode = false;
        let prefix = &self.current_word;
        let completions = self.dictionary.find_completions(prefix, 6);

        // Helper to format case matching the typed prefix
        let format_case = |word: &str| -> String {
            if self.current_word.chars().all(|c| c.is_uppercase()) && self.current_word.len() > 1 {
                word.to_uppercase()
            } else if self
                .current_word
                .chars()
                .next()
                .is_some_and(|c| c.is_uppercase())
            {
                let mut c = word.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            } else {
                word.to_string()
            }
        };

        let mut pool: Vec<String> = Vec::with_capacity(completions.len());
        for (comp, _) in completions {
            let formatted = format_case(&comp);
            if !pool.iter().any(|c| c.eq_ignore_ascii_case(&formatted)) {
                pool.push(formatted);
            }
        }

        let top_candidate = pool
            .first()
            .cloned()
            .unwrap_or_else(|| self.current_word.clone());

        let mut slots = Vec::with_capacity(3);

        if top_candidate.eq_ignore_ascii_case(&self.current_word) {
            // Case A: Typed word is already the exact top recommendation
            // Center (Slot 1) = typed word / top candidate (e.g. "how")
            // Left (Slot 0) = runner up (e.g. "however")
            // Right (Slot 2) = 3rd candidate (e.g. "how's")
            let mut left = String::new();
            let mut right = String::new();
            for c in &pool {
                if !c.eq_ignore_ascii_case(&top_candidate) {
                    if left.is_empty() {
                        left = c.clone();
                    } else if right.is_empty() && !c.eq_ignore_ascii_case(&left) {
                        right = c.clone();
                        break;
                    }
                }
            }
            slots.push(left); // Slot 0 (Left)
            slots.push(top_candidate); // Slot 1 (Center)
            slots.push(right); // Slot 2 (Right)
        } else {
            // Case B: Top candidate is an autocorrect / contraction / completion
            // e.g. typed "cant" -> top is "can't", typed "dont" -> top is "don't", typed "omw" -> top is "on my way"
            // Slot 0 (Left) = Literal string typed by user (so user can tap to keep literal)
            // Slot 1 (Center) = Top candidate (the correct punctuation / contraction / recommendation)
            // Slot 2 (Right) = Alternative completion (runner up)
            let left = self.current_word.clone();
            let center = top_candidate;
            let mut right = String::new();
            for c in &pool {
                if !c.eq_ignore_ascii_case(&left) && !c.eq_ignore_ascii_case(&center) {
                    right = c.clone();
                    break;
                }
            }
            slots.push(left); // Slot 0 (Left)
            slots.push(center); // Slot 1 (Center)
            slots.push(right); // Slot 2 (Right)
        }

        self.candidates = slots;
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphabetic() || ch == '\''
}

fn matches_preselection_context(
    pending: &PendingSelectionEcho,
    before: &str,
    words: &[String],
) -> bool {
    let history_len = pending.previous_history.len();
    if words.len() < history_len
        || !words
            .iter()
            .take(history_len)
            .zip(&pending.previous_history)
            .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    {
        return false;
    }

    if pending.previous_current_word.is_empty() {
        return words.len() == history_len;
    }

    if words.len() == history_len {
        return before.is_empty() || before.chars().last().is_some_and(char::is_whitespace);
    }

    if words.len() != history_len + 1 {
        return false;
    }

    let lagging_word = words.last().expect("length checked");
    let previous_lower = pending.previous_current_word.to_lowercase();
    let lagging_lower = lagging_word.to_lowercase();
    previous_lower.starts_with(&lagging_lower)
}

fn inline_completion_prefix(
    surrounding: &str,
    cursor_bytes: usize,
    anchor_bytes: usize,
) -> Option<&str> {
    let start = cursor_bytes.min(anchor_bytes);
    let end = cursor_bytes.max(anchor_bytes);
    if start == end
        || end != surrounding.len()
        || start == 0
        || start > surrounding.len()
        || !surrounding.is_char_boundary(start)
    {
        return None;
    }

    let before_selection = &surrounding[..start];
    let prefix = before_selection
        .rsplit(|ch: char| !ch.is_alphabetic() && ch != '\'')
        .next()?;
    (!prefix.is_empty()).then_some(prefix)
}

fn selected_text_covers_cursor(surrounding: &str, cursor_bytes: usize, selected: &str) -> bool {
    let selected_words: Vec<String> = selected
        .split(|c: char| !is_word_char(c))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if selected_words.is_empty() {
        return false;
    }

    let cursor = surrounding.floor_char_boundary(surrounding.len().min(cursor_bytes));
    let mut spans: Vec<(usize, usize, &str)> = Vec::new();
    let mut start = None;

    for (idx, ch) in surrounding.char_indices() {
        if is_word_char(ch) {
            start.get_or_insert(idx);
        } else if let Some(word_start) = start.take() {
            spans.push((word_start, idx, &surrounding[word_start..idx]));
        }
    }
    if let Some(word_start) = start {
        spans.push((word_start, surrounding.len(), &surrounding[word_start..]));
    }

    for window in spans.windows(selected_words.len()) {
        if !window
            .iter()
            .zip(&selected_words)
            .all(|((_, _, actual), expected)| actual.eq_ignore_ascii_case(expected))
        {
            continue;
        }

        let span_start = window[0].0;
        let span_end = window[window.len() - 1].1;
        if (cursor >= span_start && cursor <= span_end)
            || (cursor > span_end
                && surrounding[span_end..cursor]
                    .chars()
                    .all(|ch| !is_word_char(ch)))
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_suggestions() {
        let mut engine = SuggestEngine::new_empty();
        engine.push_char('h');
        engine.push_char('o');
        engine.push_char('w');
        assert_eq!(engine.current_word, "how");
        assert_eq!(engine.candidates.len(), 3);
        // Center slot is the top candidate "how"
        assert_eq!(engine.candidates[1], "how");
        assert!(!engine.is_next_word_mode);
    }

    #[test]
    fn test_next_word_prediction() {
        let mut engine = SuggestEngine::new_empty();
        engine.on_word_selected("how");
        assert!(engine.is_next_word_mode);
        assert_eq!(engine.candidates.len(), 3);
        assert!(
            engine.candidates.contains(&"are".to_string())
                || engine.candidates.contains(&"is".to_string())
        );

        // Next word selection: "how" -> "are"
        engine.on_word_selected("are");
        assert!(engine.is_next_word_mode);
        assert!(engine.candidates.contains(&"you".to_string()));
    }

    #[test]
    fn test_user_bigram_learning() {
        let mut engine = SuggestEngine::new_empty();
        engine.on_word_selected("hyprosk");
        engine.on_word_selected("keyboard");

        // Next time "hyprosk" is committed, "keyboard" should be predicted
        engine.on_word_selected("hyprosk");
        assert!(engine.candidates.contains(&"keyboard".to_string()));
    }

    #[test]
    fn test_backspace_editing() {
        let mut engine = SuggestEngine::new_empty();
        // Type "balls"
        for c in "balls".chars() {
            engine.push_char(c);
        }
        assert_eq!(engine.current_word, "balls");

        // Delete "lls" (3 backspaces)
        engine.pop_char();
        engine.pop_char();
        engine.pop_char();
        assert_eq!(engine.current_word, "ba");

        // Type 'l' -> "bal"
        engine.push_char('l');
        assert_eq!(engine.current_word, "bal");
        assert!(engine.candidates.iter().any(|c| c.starts_with("bal")));
    }

    #[test]
    fn test_word_selection_backspace_and_replacement() {
        let mut engine = SuggestEngine::new_empty();
        // 1. Type "up"
        engine.push_char('u');
        engine.push_char('p');
        assert_eq!(engine.current_word, "up");

        // 2. Select "update" from suggestions (sends "update ")
        engine.on_word_selected("update");
        assert_eq!(engine.current_word, "");
        assert_eq!(
            engine.word_history.last().map(|s| s.as_str()),
            Some("update")
        );

        // 3. 1st backspace deletes trailing space in text field -> restores "update"
        engine.pop_char();
        assert_eq!(engine.current_word, "update");

        // 4. 2nd backspace deletes 'e' -> "updat"
        engine.pop_char();
        assert_eq!(engine.current_word, "updat");

        // 5. 3rd backspace deletes 't' -> "upda"
        engine.pop_char();
        assert_eq!(engine.current_word, "upda");

        // Length is 4. When replacing with "updated", 4 backspaces will be sent (deleting "upda" completely).
        assert_eq!(engine.current_word.chars().count(), 4);
    }

    #[test]
    fn test_multi_word_backspacing_and_editing() {
        let mut engine = SuggestEngine::new_empty();
        // Type "hello how are you "
        for w in &["hello", "how", "are", "you"] {
            for c in w.chars() {
                engine.push_char(c);
            }
            engine.push_char(' ');
        }
        assert_eq!(engine.word_history, vec!["hello", "how", "are", "you"]);
        assert!(engine.is_next_word_mode);

        // Backspace 1: delete space after "you" -> restores "you"
        engine.pop_char();
        assert_eq!(engine.current_word, "you");
        assert_eq!(engine.word_history, vec!["hello", "how", "are"]);

        // Backspace 2, 3, 4: delete "you" completely -> predicts next for "are"
        engine.pop_char(); // "yo"
        engine.pop_char(); // "y"
        engine.pop_char(); // "" -> predicts next for "are"
        assert_eq!(engine.current_word, "");
        assert!(engine.is_next_word_mode);
        assert!(engine.candidates.contains(&"you".to_string()));

        // Backspace 5: delete space after "are" -> restores "are"
        engine.pop_char();
        assert_eq!(engine.current_word, "are");
        assert_eq!(engine.word_history, vec!["hello", "how"]);

        // Backspace 6: delete 'e' -> "ar"
        engine.pop_char();
        assert_eq!(engine.current_word, "ar");

        // Type 'm' -> "arm"
        engine.push_char('m');
        assert_eq!(engine.current_word, "arm");
        assert!(engine.candidates.iter().any(|c| c.starts_with("arm")));
    }

    #[test]
    fn test_contraction_shortcut_expansion() {
        let mut engine = SuggestEngine::new_empty();
        // Type "cant"
        for c in "cant".chars() {
            engine.push_char(c);
        }
        assert_eq!(engine.current_word, "cant");
        assert_eq!(engine.candidates.len(), 3);
        assert_eq!(engine.candidates[0], "cant"); // Left: raw literal
        assert_eq!(engine.candidates[1], "can't"); // Center: punctuation contraction
        assert_eq!(engine.candidates[2], "cannot"); // Right: alternative

        // Type "dont"
        engine.clear();
        for c in "dont".chars() {
            engine.push_char(c);
        }
        assert_eq!(engine.candidates.len(), 3);
        assert_eq!(engine.candidates[0], "dont"); // Left: raw literal
        assert_eq!(engine.candidates[1], "don't"); // Center: punctuation contraction
        assert_eq!(engine.candidates[2], "done"); // Right: alternative

        // Type "omw"
        engine.clear();
        for c in "omw".chars() {
            engine.push_char(c);
        }
        assert_eq!(engine.candidates.len(), 3);
        assert_eq!(engine.candidates[0], "omw"); // Left: raw literal
        assert_eq!(engine.candidates[1], "on my way"); // Center: shortcut expansion
        assert_eq!(engine.candidates[2], "omg"); // Right: alternative
    }

    #[test]
    fn test_slang_learning_and_permanence() {
        let mut engine = SuggestEngine::new_empty();
        // 1. Type unknown slang "hyprosk" and space
        for c in "hyprosk".chars() {
            engine.push_char(c);
        }
        engine.push_char(' ');

        // 2. It should be registered in user dictionary
        assert_eq!(engine.dictionary.user_dict.words.len(), 1);
        assert_eq!(engine.dictionary.user_dict.words[0].as_str(), "hyprosk");
        assert_eq!(engine.dictionary.user_dict.words[0].count, 1);
        assert_eq!(engine.dictionary.user_dict.words[0].is_permanent, 0);

        // 3. Type "hyprosk" again -> reinforced to permanent
        for c in "hyprosk".chars() {
            engine.push_char(c);
        }
        engine.push_char(' ');
        assert_eq!(engine.dictionary.user_dict.words[0].count, 2);
        assert_eq!(engine.dictionary.user_dict.words[0].is_permanent, 1);

        // 4. Now typing "hypr" should find "hyprosk" in candidates!
        engine.push_char('h');
        engine.push_char('y');
        engine.push_char('p');
        engine.push_char('r');
        assert!(engine.candidates.contains(&"hyprosk".to_string()));
    }

    #[test]
    fn test_backspace_undo_transient_word() {
        let mut engine = SuggestEngine::new_empty();
        // Type typo "misstype" and space
        for c in "misstype".chars() {
            engine.push_char(c);
        }
        engine.push_char(' ');
        assert_eq!(engine.dictionary.user_dict.words.len(), 1);

        // Immediately backspace -> undo reverts the transient word!
        engine.pop_char();
        assert_eq!(engine.dictionary.user_dict.words.len(), 0);
    }

    #[test]
    fn test_sync_with_text_url_boundaries() {
        let mut engine = SuggestEngine::new_empty();

        // 1. Surrounding text ends with dot: "google."
        engine.sync_with_text("google.", 7);
        assert_eq!(engine.current_word, "");
        assert!(engine.candidates.is_empty());

        // 2. Typing next letter "c" after dot -> "google.c"
        engine.sync_with_text("google.c", 8);
        assert_eq!(engine.current_word, "c");

        // 3. Complete URL "google.com/"
        engine.sync_with_text("google.com/", 11);
        assert_eq!(engine.current_word, "");
        assert!(engine.candidates.is_empty());
    }

    #[test]
    fn test_sync_ignores_stale_lagging_echo() {
        let mut engine = SuggestEngine::new_empty();
        // Type 'b' then 'r' -> current_word is 'br'.
        engine.push_char('b');
        engine.push_char('r');
        assert_eq!(engine.current_word, "br");

        // Stale Chromium echo for just 'b' arrives AFTER 'r' was typed.
        // 'b' is a strict prefix of 'br', so it must be ignored.
        engine.sync_with_text("b", 1);
        assert_eq!(
            engine.current_word, "br",
            "lagging prefix echo must not clobber current_word"
        );

        // Lagging empty echo from before the word started must not clear.
        engine.sync_with_text("", 0);
        assert_eq!(
            engine.current_word, "br",
            "lagging empty echo must not clear current_word"
        );

        // Lagging space echo from before the word started must not clear.
        engine.sync_with_text("hello ", 6);
        assert_eq!(
            engine.current_word, "br",
            "lagging space echo must not clear current_word"
        );

        // Fresh echo for the full word is accepted.
        engine.sync_with_text("br", 2);
        assert_eq!(engine.current_word, "br");

        // Forward progress to a longer word is accepted.
        engine.sync_with_text("bro", 3);
        assert_eq!(engine.current_word, "bro");

        // Unrelated word (cursor move / external edit) still syncs.
        engine.sync_with_text("xyz", 3);
        assert_eq!(engine.current_word, "xyz");
    }

    #[test]
    fn delayed_external_prefix_echo_after_selection_is_ignored() {
        let mut engine = SuggestEngine::new_empty();
        engine.sync_with_external_text("br", 2);
        assert_eq!(engine.current_word, "br");

        engine.on_word_selected("bruh");
        let selected_candidates = engine.candidates.clone();
        assert!(engine.is_next_word_mode);

        engine.sync_with_external_text("br", 2);
        assert!(engine.is_next_word_mode);
        assert!(engine.current_word.is_empty());
        assert_eq!(engine.word_history.last().map(String::as_str), Some("bruh"));
        assert_eq!(engine.candidates, selected_candidates);
        assert!(!engine
            .candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case("bruh")));

        engine.sync_with_external_text("bruh ", 5);
        assert!(engine.is_next_word_mode);
        assert_eq!(engine.word_history.last().map(String::as_str), Some("bruh"));
    }

    #[test]
    fn caret_move_into_selected_word_is_accepted() {
        let mut engine = SuggestEngine::new_empty();
        engine.sync_with_external_text("br", 2);
        engine.on_word_selected("bruh");

        engine.sync_with_external_text("bruh", 2);
        assert!(!engine.is_next_word_mode);
        assert_eq!(engine.current_word, "br");
    }

    #[test]
    fn suggestion_candidate_slots_do_not_repeat_words() {
        let mut engine = SuggestEngine::new_empty();
        engine.current_word = "cant".to_string();
        engine.recalculate();
        let nonempty: Vec<_> = engine.candidates.iter().filter(|word| !word.is_empty()).collect();
        for (idx, candidate) in nonempty.iter().enumerate() {
            assert!(!nonempty[..idx].iter().any(|previous| {
                previous.eq_ignore_ascii_case(candidate)
            }));
        }

        engine.predict_next("a");
        let nonempty: Vec<_> = engine.candidates.iter().filter(|word| !word.is_empty()).collect();
        for (idx, candidate) in nonempty.iter().enumerate() {
            assert!(!nonempty[..idx].iter().any(|previous| {
                previous.eq_ignore_ascii_case(candidate)
            }));
        }
    }
}
