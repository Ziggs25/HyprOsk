use crate::suggest::dictionary::Dictionary;

#[derive(Debug, Clone)]
pub struct SuggestEngine {
    pub dictionary: Dictionary,
    pub current_word: String,
    pub word_history: Vec<String>,
    pub candidates: Vec<String>,
    pub is_next_word_mode: bool,
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
        }
    }

    pub fn swipe_candidates(&self, path: &[char]) -> Vec<String> {
        crate::suggest::swipe::swipe_candidates(path, &self.dictionary)
    }

    pub fn push_char(&mut self, ch: char) {
        if ch.is_alphabetic() || ch == '\'' {
            if self.is_next_word_mode {
                self.is_next_word_mode = false;
                self.current_word.clear();
            }
            self.current_word.push(ch);
            self.recalculate();
        } else if ch == ' ' {
            self.on_space();
        } else if ch == '.' || ch == '?' || ch == '!' {
            if !self.current_word.is_empty() {
                let typed = std::mem::take(&mut self.current_word);
                self.word_history.push(typed);
            }
            self.word_history.clear();
            self.clear();
        } else {
            self.on_space();
        }
    }

    pub fn pop_char(&mut self) {
        if self.is_next_word_mode {
            // The cursor was right after a space following the last word.
            // This backspace deleted that trailing space from the text box.
            self.is_next_word_mode = false;
            if let Some(last) = self.word_history.pop() {
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
        let valid_len = surrounding.len().min(cursor_bytes);
        let before = &surrounding[..valid_len];
        if before.is_empty() {
            self.clear();
            return;
        }

        let words: Vec<String> = before
            .split(|c: char| !c.is_alphabetic() && c != '\'')
            .filter(|w| !w.is_empty())
            .map(|w| w.to_string())
            .collect();

        if before.ends_with(' ') {
            self.word_history = words;
            self.current_word.clear();
            if let Some(last_word) = self.word_history.last().cloned() {
                self.is_next_word_mode = true;
                self.predict_next(&last_word);
            } else {
                self.clear();
            }
        } else if let Some(last_word) = words.last().cloned() {
            self.word_history = words[..words.len() - 1].to_vec();
            self.current_word = last_word;
            self.is_next_word_mode = false;
            self.recalculate();
        } else {
            self.clear();
        }
    }

    pub fn on_space(&mut self) {
        if !self.current_word.is_empty() {
            let typed = std::mem::take(&mut self.current_word);
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

    pub fn on_word_selected(&mut self, chosen_word: &str) {
        let chosen_clean = chosen_word.trim().to_string();
        if let Some(prev) = self.word_history.last() {
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
        self.candidates = predictions.into_iter().map(|(w, _)| w).collect();
    }

    pub fn clear(&mut self) {
        self.current_word.clear();
        self.word_history.clear();
        self.candidates.clear();
        self.is_next_word_mode = false;
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
        let completions = self.dictionary.find_completions(prefix, 4);

        let mut slots = Vec::with_capacity(3);

        // Slot 1 (Left): Literal string typed so far
        slots.push(self.current_word.clone());

        // Slot 2 (Center - Best match) & Slot 3 (Right)
        for (comp, _) in completions {
            if !slots.iter().any(|s| s.eq_ignore_ascii_case(&comp)) {
                // Preserve capitalization if typed in uppercase
                let formatted = if self.current_word.chars().next().map_or(false, |c| c.is_uppercase()) {
                    let mut c = comp.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                } else {
                    comp
                };
                slots.push(formatted);
                if slots.len() >= 3 {
                    break;
                }
            }
        }

        self.candidates = slots;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_suggestions() {
        let mut engine = SuggestEngine::new();
        engine.push_char('h');
        engine.push_char('o');
        engine.push_char('w');
        assert_eq!(engine.current_word, "how");
        assert!(!engine.candidates.is_empty());
        assert_eq!(engine.candidates[0], "how");
        assert!(!engine.is_next_word_mode);
    }

    #[test]
    fn test_next_word_prediction() {
        let mut engine = SuggestEngine::new();
        engine.on_word_selected("how");
        assert!(engine.is_next_word_mode);
        assert!(!engine.candidates.is_empty());
        assert!(engine.candidates.contains(&"are".to_string()) || engine.candidates.contains(&"is".to_string()));

        // Next word selection: "how" -> "are"
        engine.on_word_selected("are");
        assert!(engine.is_next_word_mode);
        assert!(engine.candidates.contains(&"you".to_string()));
    }

    #[test]
    fn test_user_bigram_learning() {
        let mut engine = SuggestEngine::new();
        engine.on_word_selected("hyprosk");
        engine.on_word_selected("keyboard");

        // Next time "hyprosk" is committed, "keyboard" should be predicted
        engine.on_word_selected("hyprosk");
        assert!(engine.candidates.contains(&"keyboard".to_string()));
    }

    #[test]
    fn test_backspace_editing() {
        let mut engine = SuggestEngine::new();
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
        let mut engine = SuggestEngine::new();
        // 1. Type "up"
        engine.push_char('u');
        engine.push_char('p');
        assert_eq!(engine.current_word, "up");

        // 2. Select "update" from suggestions (sends "update ")
        engine.on_word_selected("update");
        assert_eq!(engine.current_word, "");
        assert_eq!(engine.word_history.last().map(|s| s.as_str()), Some("update"));

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
        let mut engine = SuggestEngine::new();
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
}
