use crate::suggest::dictionary::Dictionary;

#[derive(Debug, Clone)]
pub struct SuggestEngine {
    dictionary: Dictionary,
    pub current_word: String,
    pub candidates: Vec<String>,
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
            candidates: Vec::new(),
        }
    }

    pub fn swipe_candidates(&self, path: &[char]) -> Vec<String> {
        crate::suggest::swipe::swipe_candidates(path, &self.dictionary)
    }

    pub fn push_char(&mut self, ch: char) {
        if ch.is_alphabetic() || ch == '\'' {
            self.current_word.push(ch);
            self.recalculate();
        } else {
            self.clear();
        }
    }

    pub fn pop_char(&mut self) {
        self.current_word.pop();
        self.recalculate();
    }

    pub fn clear(&mut self) {
        self.current_word.clear();
        self.candidates.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.current_word.is_empty()
    }

    pub fn recalculate(&mut self) {
        if self.current_word.is_empty() {
            self.candidates.clear();
            return;
        }

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
