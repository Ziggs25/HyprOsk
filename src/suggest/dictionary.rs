use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnigramEntry {
    pub offset: u32,
    pub len: u16,
    pub freq: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigramEntry {
    pub src_idx: u32,
    pub dst_idx: u32,
    pub freq: u16,
}

#[derive(Debug, Clone)]
pub struct Dictionary {
    words_blob: String,
    unigrams: Vec<UnigramEntry>,
    bigrams: Vec<BigramEntry>,
    user_bigrams: HashMap<(String, String), u32>,
}

impl Default for Dictionary {
    fn default() -> Self {
        Self::new()
    }
}

impl Dictionary {
    pub fn new() -> Self {
        let mut dict = Self {
            words_blob: String::with_capacity(96 * 1024),
            unigrams: Vec::with_capacity(10000),
            bigrams: Vec::with_capacity(2000),
            user_bigrams: HashMap::new(),
        };
        dict.load_default_corpus();
        dict
    }

    #[inline]
    pub fn get_word(&self, entry: &UnigramEntry) -> &str {
        let start = entry.offset as usize;
        let end = start + entry.len as usize;
        &self.words_blob[start..end]
    }

    pub fn find_word_idx(&self, word: &str) -> Option<usize> {
        let target = word.to_lowercase();
        self.unigrams.binary_search_by(|entry| {
            let w = self.get_word(entry);
            w.cmp(&target)
        }).ok()
    }

    pub fn all_words(&self, limit: usize) -> Vec<(String, u32)> {
        let mut words: Vec<(String, u32)> = self.unigrams.iter()
            .map(|e| (self.get_word(e).to_string(), e.freq as u32))
            .collect();
        words.sort_by(|a, b| b.1.cmp(&a.1));
        words.truncate(limit);
        words
    }

    pub fn find_completions(&self, prefix: &str, limit: usize) -> Vec<(String, u32)> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let prefix_lower = prefix.to_lowercase();
        let prefix_bytes = prefix_lower.as_bytes();

        let start_idx = match self.unigrams.binary_search_by(|entry| {
            let w = self.get_word(entry);
            if w.starts_with(&prefix_lower) {
                std::cmp::Ordering::Greater
            } else {
                w.as_bytes().cmp(prefix_bytes)
            }
        }) {
            Ok(i) => i,
            Err(i) => i,
        };

        let mut matches = Vec::new();
        for entry in &self.unigrams[start_idx..] {
            let w = self.get_word(entry);
            if !w.starts_with(&prefix_lower) {
                break;
            }
            matches.push((w.to_string(), entry.freq as u32));
        }

        matches.sort_by(|a, b| b.1.cmp(&a.1));
        matches.truncate(limit);
        matches
    }

    pub fn predict_next_words(&self, prev_word: &str, limit: usize) -> Vec<(String, u32)> {
        let prev_lower = prev_word.trim().to_lowercase();
        if prev_lower.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<(String, u32)> = Vec::with_capacity(limit * 2);

        // 1. Check user-learned dynamic bigrams first (highest priority)
        for ((w1, w2), freq) in &self.user_bigrams {
            if w1.eq_ignore_ascii_case(&prev_lower) {
                results.push((w2.clone(), *freq + 10000));
            }
        }

        // 2. Query static compiled bigrams via binary search
        if let Some(src_idx) = self.find_word_idx(&prev_lower) {
            let src_u32 = src_idx as u32;
            let start = match self.bigrams.binary_search_by_key(&src_u32, |b| b.src_idx) {
                Ok(mut idx) => {
                    while idx > 0 && self.bigrams[idx - 1].src_idx == src_u32 {
                        idx -= 1;
                    }
                    idx
                }
                Err(_) => usize::MAX,
            };

            if start != usize::MAX {
                for b in &self.bigrams[start..] {
                    if b.src_idx != src_u32 {
                        break;
                    }
                    if let Some(dst_entry) = self.unigrams.get(b.dst_idx as usize) {
                        let dst_word = self.get_word(dst_entry);
                        if !results.iter().any(|(w, _)| w.eq_ignore_ascii_case(dst_word)) {
                            results.push((dst_word.to_string(), b.freq as u32));
                        }
                    }
                }
            }
        }

        // 3. Fallback connector words if fewer than limit results
        if results.len() < limit {
            let fallbacks = ["the", "to", "and", "is", "a", "it", "you", "for", "in", "of", "with", "this", "that"];
            for f in fallbacks {
                if !f.eq_ignore_ascii_case(&prev_lower) && !results.iter().any(|(w, _)| w.eq_ignore_ascii_case(f)) {
                    results.push((f.to_string(), 100));
                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }

        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.truncate(limit);
        results
    }

    pub fn record_user_bigram(&mut self, prev_word: &str, next_word: &str) {
        let w1 = prev_word.trim().to_lowercase();
        let w2 = next_word.trim().to_lowercase();
        if w1.is_empty() || w2.is_empty() || w1 == w2 {
            return;
        }
        let count = self.user_bigrams.entry((w1, w2)).or_insert(0);
        *count = count.saturating_add(1);
    }

    fn load_default_corpus(&mut self) {
        let raw_wordlist = if let Ok(c) = std::fs::read_to_string("assets/google-10000-english.txt") {
            c
        } else {
            include_str!("../../assets/google-10000-english.txt").to_string()
        };

        let mut word_freq_map: HashMap<String, u16> = HashMap::with_capacity(10500);

        let mut rank = 0u16;
        for line in raw_wordlist.lines() {
            let w = line.trim().to_lowercase();
            if w.is_empty() || (w.len() <= 1 && w != "a" && w != "i") {
                continue;
            }
            let freq = 10000u16.saturating_sub(rank).saturating_add(100);
            word_freq_map.insert(w, freq);
            rank += 1;
        }

        let tech_words = [
            ("hyprland", 4000), ("hyprosk", 5000), ("wayland", 3800), ("nixos", 4500),
            ("linux", 3900), ("rust", 3700), ("cargo", 3600), ("github", 3500),
            ("terminal", 3200), ("systemd", 3000), ("systemctl", 3000), ("heliboard", 3500),
            ("keyboard", 3200), ("thanks", 4200), ("thank", 4000), ("welcome", 3800),
            ("please", 3900), ("sure", 3600), ("great", 3500), ("awesome", 3400),
            ("bro", 4500),
        ];
        for (w, f) in tech_words {
            word_freq_map.insert(w.to_string(), f);
        }

        let mut sorted_words: Vec<(String, u16)> = word_freq_map.into_iter().collect();
        sorted_words.sort_by(|a, b| a.0.cmp(&b.0));

        self.words_blob.clear();
        self.unigrams.clear();
        let mut word_to_idx: HashMap<String, u32> = HashMap::with_capacity(sorted_words.len());

        for (idx, (word, freq)) in sorted_words.into_iter().enumerate() {
            let offset = self.words_blob.len() as u32;
            let len = word.len() as u16;
            self.words_blob.push_str(&word);
            self.unigrams.push(UnigramEntry { offset, len, freq });
            word_to_idx.insert(word, idx as u32);
        }

        let bigram_pairs: &[(&str, &str, u16)] = &[
            // English conversations
            ("how", "are", 980), ("how", "is", 950), ("how", "to", 920), ("how", "much", 890), ("how", "do", 870), ("how", "can", 850),
            ("are", "you", 990), ("are", "we", 880), ("are", "there", 860), ("are", "they", 840),
            ("thank", "you", 1000), ("thanks", "for", 980), ("thanks", "bro", 950), ("thanks", "a", 900),
            ("you", "are", 980), ("you", "can", 960), ("you", "have", 940), ("you", "know", 920), ("you", "want", 900),
            ("you're", "welcome", 990), ("no", "problem", 980), ("no", "worries", 960),
            ("what", "is", 980), ("what", "are", 950), ("what", "do", 930), ("what", "about", 910), ("what", "time", 890),
            ("where", "is", 970), ("where", "are", 950), ("where", "can", 920),
            ("when", "is", 960), ("when", "you", 940), ("when", "will", 920),
            ("why", "is", 960), ("why", "are", 940), ("why", "not", 920), ("why", "do", 900),
            ("i", "am", 990), ("i", "have", 980), ("i", "will", 970), ("i", "want", 960), ("i", "need", 950), ("i", "think", 940), ("i", "can", 930), ("i", "would", 920), ("i", "know", 910),
            ("can", "you", 990), ("can", "i", 970), ("can", "we", 950), ("can", "be", 930),
            ("could", "you", 980), ("would", "you", 980), ("should", "be", 950),
            ("let", "me", 990), ("let", "us", 960), ("let", "you", 930),
            ("it", "is", 990), ("it", "was", 970), ("it", "will", 950), ("it", "would", 930), ("it", "can", 910),
            ("there", "is", 980), ("there", "are", 970), ("there", "will", 940),
            ("this", "is", 990), ("this", "was", 960), ("this", "will", 940), ("this", "one", 920),
            ("that", "is", 990), ("that", "was", 970), ("that", "would", 950), ("that", "you", 930),
            ("the", "same", 990), ("the", "best", 980), ("the", "first", 970), ("the", "world", 960), ("the", "way", 950), ("the", "system", 940), ("the", "file", 930), ("the", "user", 920),
            ("to", "the", 990), ("to", "be", 980), ("to", "make", 970), ("to", "get", 960), ("to", "do", 950), ("to", "see", 940), ("to", "have", 930),
            ("of", "the", 990), ("of", "this", 960), ("of", "course", 950), ("of", "a", 940),
            ("in", "the", 990), ("in", "this", 970), ("in", "a", 960), ("in", "my", 950), ("in", "our", 940),
            ("for", "the", 990), ("for", "you", 980), ("for", "this", 970), ("for", "a", 960), ("for", "me", 950),
            ("on", "the", 990), ("on", "this", 970), ("on", "your", 960), ("on", "it", 950),
            ("with", "the", 990), ("with", "you", 980), ("with", "this", 970), ("with", "a", 960), ("with", "me", 950),
            ("at", "the", 990), ("at", "all", 970), ("at", "least", 960), ("at", "home", 950),
            ("from", "the", 990), ("from", "this", 970), ("from", "here", 960), ("from", "scratch", 950),
            ("by", "the", 990), ("by", "default", 980), ("by", "this", 960),
            ("about", "the", 990), ("about", "this", 970), ("about", "it", 960), ("about", "that", 950),
            ("be", "able", 980), ("be", "sure", 970), ("be", "the", 960), ("be", "fine", 950),
            ("have", "a", 990), ("have", "to", 980), ("have", "been", 970), ("have", "the", 960), ("have", "any", 950),
            ("do", "you", 990), ("do", "it", 980), ("do", "not", 970), ("do", "this", 960),
            ("make", "sure", 990), ("make", "it", 980), ("make", "sense", 970), ("make", "a", 960),
            ("get", "the", 980), ("get", "it", 970), ("get", "back", 960), ("get", "started", 950),
            ("bro", "thanks", 990), ("bro", "can", 980), ("bro", "please", 970), ("bro", "what", 960), ("bro", "how", 950), ("bro", "i", 940),
            ("looks", "good", 990), ("looks", "great", 980), ("looks", "like", 970), ("looks", "awesome", 960),
            ("look", "at", 980), ("look", "into", 970), ("look", "for", 960), ("look", "good", 950),
            ("drive", "to", 980), ("drive", "the", 970), ("drive", "safe", 960), ("drive", "home", 950),
            ("browse", "the", 980), ("browse", "web", 970), ("browse", "through", 960),
            ("test", "it", 990), ("test", "the", 980), ("test", "first", 970), ("test", "build", 960),
            ("commit", "it", 990), ("commit", "the", 980), ("commit", "to", 970), ("commit", "message", 960),
            ("push", "to", 990), ("push", "it", 980), ("push", "origin", 970), ("push", "the", 960),
            ("check", "out", 990), ("check", "the", 980), ("check", "it", 970), ("check", "this", 960),
            ("switch", "to", 990), ("switch", "the", 980), ("switch", "branch", 970),
            ("rebuild", "switch", 990), ("rebuild", "test", 980), ("rebuild", "the", 970),
            ("hello", "bro", 980), ("hello", "there", 970), ("hello", "world", 960),
            ("good", "morning", 980), ("good", "night", 970), ("good", "job", 960), ("good", "luck", 950), ("good", "to", 940),
            ("see", "you", 990), ("looking", "good", 970), ("sounds", "good", 960),
            ("please", "let", 980), ("please", "check", 970), ("please", "help", 960),
            ("as", "well", 980), ("as", "soon", 970), ("as", "possible", 960),

            // Linux / NixOS / Hyprland bigrams
            ("git", "status", 990), ("git", "commit", 980), ("git", "push", 970), ("git", "pull", 960), ("git", "diff", 950), ("git", "checkout", 940), ("git", "log", 930),
            ("cargo", "build", 990), ("cargo", "check", 980), ("cargo", "test", 970), ("cargo", "run", 960),
            ("nixos", "rebuild", 990), ("nixos-rebuild", "switch", 990), ("nixos-rebuild", "test", 980),
            ("sudo", "nixos-rebuild", 990), ("sudo", "systemctl", 980),
            ("systemctl", "status", 990), ("systemctl", "restart", 980), ("systemctl", "start", 970), ("systemctl", "stop", 960),
            ("hyprland", "config", 980), ("hyprosk", "daemon", 980),
        ];

        self.bigrams.clear();
        for &(w1, w2, freq) in bigram_pairs {
            if let (Some(&src_idx), Some(&dst_idx)) = (word_to_idx.get(w1), word_to_idx.get(w2)) {
                self.bigrams.push(BigramEntry { src_idx, dst_idx, freq });
            }
        }

        self.bigrams.sort_by(|a, b| {
            a.src_idx.cmp(&b.src_idx).then_with(|| b.freq.cmp(&a.freq))
        });
    }
}
