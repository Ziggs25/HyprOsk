use crate::suggest::dictionary::Dictionary;

#[derive(Debug, Clone)]
pub struct SuggestEngine {
    pub dictionary: Dictionary,
    pub current_word: String,
    pub last_committed_word: Option<String>,
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
            last_committed_word: None,
            candidates: Vec::new(),
            is_next_word_mode: false,
        }
    }

    pub fn swipe_candidates(&self, path: &[char]) -> Vec<String> {
        crate::suggest::swipe::swipe_candidates(path, &self.dictionary)
    }

    pub fn push_char(&mut self, ch: char) {
        if ch.is_alphabetic() || ch == '\'' {
            self.is_next_word_mode = false;
            self.current_word.push(ch);
            self.recalculate();
        } else if ch == ' ' {
            self.on_space();
        } else {
            self.clear();
        }
    }

    pub fn pop_char(&mut self) {
        self.is_next_word_mode = false;
        self.current_word.pop();
        if self.current_word.is_empty() {
            self.candidates.clear();
            self.last_committed_word = None;
        } else {
            self.recalculate();
        }
    }

    pub fn on_space(&mut self) {
        if !self.current_word.is_empty() {
            let typed = std::mem::take(&mut self.current_word);
            if let Some(prev) = self.last_committed_word.take() {
                self.dictionary.record_user_bigram(&prev, &typed);
            }
            self.last_committed_word = Some(typed.clone());
            self.predict_next(&typed);
        } else if let Some(prev) = self.last_committed_word.as_ref().cloned() {
            self.predict_next(&prev);
        } else {
            self.clear();
        }
    }

    pub fn on_word_selected(&mut self, chosen_word: &str) {
        let chosen_clean = chosen_word.trim().to_string();
        if let Some(prev) = self.last_committed_word.take() {
            self.dictionary.record_user_bigram(&prev, &chosen_clean);
        }
        self.last_committed_word = Some(chosen_clean.clone());
        self.current_word.clear();
        self.predict_next(&chosen_clean);
    }

    fn predict_next(&mut self, prev_word: &str) {
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
        self.last_committed_word = None;
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
}
