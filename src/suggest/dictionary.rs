use std::collections::HashMap;
use crate::suggest::user_dict::{find_shortcut_details, UserDictionary};

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
    pub user_dict: UserDictionary,
}

impl Default for Dictionary {
    fn default() -> Self {
        Self::new()
    }
}

struct TopCollector<'a> {
    cap: usize,
    items: [(&'a str, u32); 8],
    count: usize,
}

impl<'a> TopCollector<'a> {
    #[inline]
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.min(8),
            items: [("", 0); 8],
            count: 0,
        }
    }

    #[inline]
    fn insert(&mut self, word: &'a str, score: u32) {
        for i in 0..self.count {
            if self.items[i].0.eq_ignore_ascii_case(word) {
                if score > self.items[i].1 {
                    self.items[i].1 = score;
                }
                return;
            }
        }
        if self.count < self.cap {
            self.items[self.count] = (word, score);
            self.count += 1;
            let mut j = self.count - 1;
            while j > 0 && self.items[j].1 > self.items[j - 1].1 {
                self.items.swap(j, j - 1);
                j -= 1;
            }
        } else if score > self.items[self.cap - 1].1 {
            self.items[self.cap - 1] = (word, score);
            let mut j = self.cap - 1;
            while j > 0 && self.items[j].1 > self.items[j - 1].1 {
                self.items.swap(j, j - 1);
                j -= 1;
            }
        }
    }
}

impl Dictionary {
    pub fn new() -> Self {
        let mut dict = Self {
            words_blob: String::with_capacity(96 * 1024),
            unigrams: Vec::with_capacity(10000),
            bigrams: Vec::with_capacity(2000),
            user_dict: UserDictionary::new(),
        };
        dict.load_default_corpus();
        dict
    }

    /// Creates a dictionary with empty user dictionary (isolated from disk).
    pub fn new_empty() -> Self {
        let mut dict = Self {
            words_blob: String::with_capacity(96 * 1024),
            unigrams: Vec::with_capacity(10000),
            bigrams: Vec::with_capacity(2000),
            user_dict: UserDictionary::new_empty(),
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
        words.sort_by_key(|b| std::cmp::Reverse(b.1));
        words.truncate(limit);
        words
    }

    /// Fast, zero-heap-allocation prefix scanning on hot path.
    /// Combines shortcut expansions, user history, and base vocabulary.
    pub fn find_completions(&self, prefix: &str, limit: usize) -> Vec<(String, u32)> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let prefix_lower = prefix.to_lowercase();
        let prefix_bytes = prefix_lower.as_bytes();

        let mut collector = TopCollector::new(limit.max(8));

        // 1. Shortcut check (e.g. 'cant' -> "can't", 'omw' -> "on my way") with alternatives
        if let Some((shortcut, alternatives)) = find_shortcut_details(&prefix_lower) {
            collector.insert(shortcut, 100_000);
            for (idx, alt) in alternatives.iter().enumerate() {
                collector.insert(alt, 90_000u32.saturating_sub((idx as u32) * 1_000));
            }
        }

        // 2. Query User History Dictionary
        let mut user_candidates: Vec<(&str, u32)> = Vec::with_capacity(16);
        self.user_dict.find_completions(&prefix_lower, &mut user_candidates);
        for (w, score) in user_candidates {
            collector.insert(w, score);
        }

        // 3. Binary search base dictionary range
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

        // Scan consecutive matching unigrams with zero string heap allocations
        for entry in &self.unigrams[start_idx..] {
            let w = self.get_word(entry);
            if !w.starts_with(&prefix_lower) {
                break;
            }
            // Exact full-word match gets priority over longer prefix completions
            let score = if w == prefix_lower {
                (entry.freq as u32) + 2000
            } else {
                entry.freq as u32
            };
            collector.insert(w, score);
        }

        // 4. Fallback search if fewer than 3 candidates:
        // Shorten prefix by 1 character to find related stem completions
        if collector.count < 3 && prefix_lower.len() > 1 {
            let short_prefix = &prefix_lower[..prefix_lower.len() - 1];
            let short_bytes = short_prefix.as_bytes();
            let short_start = match self.unigrams.binary_search_by(|entry| {
                let w = self.get_word(entry);
                if w.starts_with(short_prefix) {
                    std::cmp::Ordering::Greater
                } else {
                    w.as_bytes().cmp(short_bytes)
                }
            }) {
                Ok(i) => i,
                Err(i) => i,
            };

            for entry in &self.unigrams[short_start..] {
                let w = self.get_word(entry);
                if !w.starts_with(short_prefix) {
                    break;
                }
                collector.insert(w, (entry.freq as u32) / 2);
                if collector.count >= 6 {
                    break;
                }
            }
        }

        // 5. Final fallback if still fewer than 3 candidates
        if collector.count < 3 {
            let fallbacks = ["the", "to", "and", "you", "is", "a", "it", "in", "for"];
            for f in fallbacks {
                collector.insert(f, 50);
                if collector.count >= 3 {
                    break;
                }
            }
        }

        let mut results = Vec::with_capacity(collector.count);
        for i in 0..collector.count {
            results.push((collector.items[i].0.to_string(), collector.items[i].1));
        }
        results
    }

    pub fn predict_next_words(&self, prev_word: &str, limit: usize) -> Vec<(String, u32)> {
        let prev_lower = prev_word.trim().to_lowercase();
        if prev_lower.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<(String, u32)> = Vec::with_capacity(limit * 2);

        // 1. Check user-learned dynamic bigrams first (highest priority)
        let mut user_next = Vec::with_capacity(8);
        self.user_dict.predict_next(&prev_lower, &mut user_next);
        for (w, score) in user_next {
            results.push((w.to_string(), score));
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

        results.sort_by_key(|b| std::cmp::Reverse(b.1));
        results.truncate(limit);
        results
    }

    #[inline]
    pub fn record_user_word(&mut self, word: &str, is_explicit_selection: bool) {
        self.user_dict.record_word(word, is_explicit_selection);
    }

    #[inline]
    pub fn record_user_bigram(&mut self, prev_word: &str, next_word: &str) {
        self.user_dict.record_bigram(prev_word, next_word);
    }

    #[inline]
    pub fn revert_last_user_word(&mut self, word: &str) {
        self.user_dict.revert_last_word(word);
    }

    #[inline]
    pub fn flush_user_dict(&mut self) {
        self.user_dict.flush_if_dirty();
    }

    pub fn load_binary_corpus(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() < 32 {
            return Err("Binary dictionary too small");
        }
        if &data[0..8] != b"HYPROSK\0" {
            return Err("Invalid dictionary magic");
        }
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version != 1 {
            return Err("Unsupported dictionary version");
        }
        let word_count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        let bigram_count = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;
        let string_pool_len = u32::from_le_bytes(data[20..24].try_into().unwrap()) as usize;

        let str_start = 32;
        let str_end = str_start + string_pool_len;
        if data.len() < str_end {
            return Err("Truncated string pool");
        }
        let str_blob = std::str::from_utf8(&data[str_start..str_end]).map_err(|_| "Invalid UTF-8 in string pool")?;

        let unigram_start = str_end;
        let unigram_end = unigram_start + word_count * 8;
        if data.len() < unigram_end {
            return Err("Truncated unigram table");
        }

        let mut unigrams = Vec::with_capacity(word_count);
        for i in 0..word_count {
            let offset_pos = unigram_start + i * 8;
            let offset = u32::from_le_bytes(data[offset_pos..offset_pos + 4].try_into().unwrap());
            let len = u16::from_le_bytes(data[offset_pos + 4..offset_pos + 6].try_into().unwrap());
            let freq = u16::from_le_bytes(data[offset_pos + 6..offset_pos + 8].try_into().unwrap());
            unigrams.push(UnigramEntry { offset, len, freq });
        }

        let bigram_start = unigram_end;
        let bigram_end = bigram_start + bigram_count * 10;
        if data.len() < bigram_end {
            return Err("Truncated bigram table");
        }

        let mut bigrams = Vec::with_capacity(bigram_count);
        for i in 0..bigram_count {
            let b_pos = bigram_start + i * 10;
            let src_idx = u32::from_le_bytes(data[b_pos..b_pos + 4].try_into().unwrap());
            let dst_idx = u32::from_le_bytes(data[b_pos + 4..b_pos + 8].try_into().unwrap());
            let freq = u16::from_le_bytes(data[b_pos + 8..b_pos + 10].try_into().unwrap());
            bigrams.push(BigramEntry { src_idx, dst_idx, freq });
        }

        self.words_blob = str_blob.to_string();
        self.unigrams = unigrams;
        self.bigrams = bigrams;
        Ok(())
    }

    fn load_default_corpus(&mut self) {
        // 1. Try pre-compiled HeliBoard AOSP binary dictionary (zero runtime overhead)
        static EMBEDDED_DICT: &[u8] = include_bytes!("../../assets/en_us.hyprosk.dict");
        if self.load_binary_corpus(EMBEDDED_DICT).is_ok() {
            tracing::info!(
                "Loaded HeliBoard AOSP binary dictionary: {} words, {} bigrams",
                self.unigrams.len(),
                self.bigrams.len()
            );
            return;
        }

        // 2. Fallback to google-10000 text list if needed
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

        let extra_words = [
            ("hyprland", 4000), ("hyprosk", 5000), ("wayland", 3800), ("nixos", 4500),
            ("linux", 3900), ("rust", 3700), ("cargo", 3600), ("github", 3500),
            ("terminal", 3200), ("systemd", 3000), ("systemctl", 3000), ("heliboard", 3500),
            ("keyboard", 3200), ("thanks", 4200), ("thank", 4000), ("welcome", 3800),
            ("please", 3900), ("sure", 3600), ("great", 3500), ("awesome", 3400),
            ("bro", 4500), ("doing", 3800), ("today", 3700), ("working", 3800),
            ("going", 3800), ("ready", 3700), ("fine", 3600), ("idea", 3400),
            ("feature", 3500), ("support", 3400), ("update", 3600), ("feeling", 3400),
        ];
        for (w, f) in extra_words {
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
            // Question Chains
            ("how", "are", 990), ("how", "is", 960), ("how", "can", 930), ("how", "to", 900), ("how", "much", 870), ("how", "do", 850),
            ("are", "you", 995), ("are", "we", 920), ("are", "there", 890), ("are", "they", 860),
            ("you", "doing", 990), ("you", "ready", 970), ("you", "know", 950), ("you", "can", 930), ("you", "have", 910), ("you", "want", 890),
            ("doing", "today", 990), ("doing", "well", 970), ("doing", "good", 950), ("doing", "fine", 930),
            ("today", "and", 980), ("today", "bro", 960), ("today", "with", 940),
            ("what", "is", 990), ("what", "do", 960), ("what", "are", 930), ("what", "about", 900), ("what", "time", 870),
            ("where", "is", 990), ("where", "are", 960), ("where", "can", 930), ("where", "do", 900),
            ("when", "is", 990), ("when", "will", 960), ("when", "you", 930), ("when", "can", 900),
            ("why", "is", 990), ("why", "not", 960), ("why", "are", 930), ("why", "do", 900),
            ("who", "is", 990), ("who", "are", 960), ("who", "was", 930),
            ("can", "you", 995), ("can", "we", 960), ("can", "i", 940), ("can", "be", 920),

            // Action / Request Chains
            ("please", "let", 995), ("please", "help", 970), ("please", "check", 950), ("please", "see", 930), ("please", "make", 910),
            ("let", "me", 995), ("let", "us", 960), ("let", "them", 930), ("let", "it", 900),
            ("me", "know", 995), ("me", "check", 960), ("me", "see", 940), ("me", "have", 920), ("me", "try", 900),
            ("know", "if", 995), ("know", "what", 960), ("know", "when", 930), ("know", "how", 900), ("know", "more", 870),
            ("if", "you", 995), ("if", "there", 960), ("if", "it", 930), ("if", "possible", 900),
            ("possible", "to", 990), ("possible", "for", 960), ("possible", "thanks", 930),

            // Greeting & Gratitude Chains
            ("thank", "you", 1000), ("thank", "god", 950), ("thank", "everyone", 920),
            ("thanks", "for", 995), ("thanks", "bro", 980), ("thanks", "a", 950), ("thanks", "again", 920),
            ("for", "the", 995), ("for", "your", 975), ("for", "this", 950), ("for", "all", 930), ("for", "me", 910),
            ("the", "help", 990), ("the", "update", 970), ("the", "new", 950), ("the", "system", 930), ("the", "code", 910), ("the", "best", 890),
            ("help", "with", 980), ("help", "and", 960), ("help", "bro", 940),
            ("good", "morning", 990), ("good", "job", 975), ("good", "luck", 955), ("good", "to", 935), ("good", "night", 915),
            ("looking", "good", 990), ("looking", "great", 970), ("looking", "forward", 950), ("looking", "at", 930),
            ("looks", "good", 995), ("looks", "great", 975), ("looks", "awesome", 955), ("looks", "like", 935),
            ("see", "you", 995), ("see", "if", 960), ("see", "what", 930),
            ("sounds", "good", 990), ("sounds", "great", 970), ("sounds", "awesome", 950),
            ("you're", "welcome", 995), ("no", "problem", 990), ("no", "worries", 970),

            // Statement Chains
            ("i", "am", 995), ("i", "will", 980), ("i", "have", 965), ("i", "think", 950), ("i", "want", 935), ("i", "need", 920), ("i", "can", 905),
            ("am", "going", 990), ("am", "ready", 970), ("am", "working", 950), ("am", "trying", 930), ("am", "done", 910),
            ("going", "to", 995), ("going", "home", 960), ("going", "back", 930), ("going", "well", 900),
            ("will", "be", 995), ("will", "do", 965), ("will", "check", 940), ("will", "make", 920), ("will", "test", 900),
            ("be", "ready", 990), ("be", "there", 970), ("be", "fine", 950), ("be", "able", 930), ("be", "great", 910),
            ("have", "a", 995), ("have", "to", 980), ("have", "been", 960), ("have", "done", 940), ("have", "any", 920),
            ("think", "that", 990), ("think", "about", 965), ("think", "it", 940), ("think", "so", 920), ("think", "we", 900),
            ("want", "to", 995), ("want", "you", 960), ("want", "the", 930), ("want", "more", 900),
            ("need", "to", 995), ("need", "help", 960), ("need", "more", 930), ("need", "some", 900),
            ("this", "is", 995), ("this", "looks", 975), ("this", "one", 950), ("this", "will", 930), ("this", "was", 910),
            ("that", "is", 995), ("that", "looks", 970), ("that", "would", 950), ("that", "was", 930), ("that", "you", 910),
            ("it", "is", 995), ("it", "works", 975), ("it", "looks", 955), ("it", "was", 935), ("it", "will", 915),
            ("is", "working", 990), ("is", "good", 970), ("is", "ready", 950), ("is", "a", 930), ("is", "done", 910),
            ("working", "fine", 990), ("working", "great", 970), ("working", "well", 950), ("working", "on", 930),
            ("fine", "now", 980), ("fine", "thanks", 960), ("fine", "bro", 940),
            ("great", "job", 990), ("great", "work", 970), ("great", "idea", 950), ("great", "feature", 930),
            ("ready", "for", 990), ("ready", "to", 970), ("ready", "now", 950),
            ("done", "with", 990), ("done", "now", 970), ("done", "thanks", 950),

            // Prepositions & Connectors
            ("to", "the", 995), ("to", "be", 980), ("to", "make", 960), ("to", "get", 940), ("to", "do", 920), ("to", "see", 900),
            ("of", "the", 995), ("of", "this", 970), ("of", "course", 950), ("of", "a", 930),
            ("in", "the", 995), ("in", "this", 970), ("in", "a", 950), ("in", "my", 930),
            ("on", "the", 995), ("on", "this", 970), ("on", "your", 950), ("on", "it", 930),
            ("with", "the", 995), ("with", "you", 975), ("with", "this", 950), ("with", "a", 930),
            ("at", "the", 995), ("at", "all", 970), ("at", "least", 950), ("at", "home", 930),
            ("from", "the", 995), ("from", "this", 970), ("from", "here", 950),
            ("by", "the", 995), ("by", "default", 975), ("by", "this", 950),
            ("about", "the", 995), ("about", "this", 970), ("about", "it", 950), ("about", "that", 930),
            ("as", "well", 995), ("as", "soon", 975), ("as", "possible", 955),

            // User / Developer / Linux Chains
            ("bro", "thanks", 995), ("bro", "can", 980), ("bro", "please", 965), ("bro", "what", 950), ("bro", "how", 935), ("bro", "i", 920),
            ("check", "out", 990), ("check", "the", 970), ("check", "it", 950), ("check", "this", 930),
            ("test", "it", 990), ("test", "the", 970), ("test", "first", 950), ("test", "build", 930),
            ("commit", "it", 990), ("commit", "the", 975), ("commit", "to", 955), ("commit", "changes", 935),
            ("push", "to", 990), ("push", "it", 970), ("push", "origin", 950), ("push", "the", 930),
            ("switch", "to", 990), ("switch", "the", 970), ("switch", "branch", 950),
            ("rebuild", "switch", 990), ("rebuild", "test", 970), ("rebuild", "the", 950),
            ("hello", "bro", 990), ("hello", "there", 970), ("hello", "world", 950),
            ("git", "status", 995), ("git", "commit", 985), ("git", "push", 975), ("git", "pull", 965), ("git", "diff", 955),
            ("cargo", "build", 995), ("cargo", "check", 985), ("cargo", "test", 975), ("cargo", "run", 965),
            ("nixos", "rebuild", 995), ("nixos-rebuild", "switch", 995), ("nixos-rebuild", "test", 980),
            ("sudo", "nixos-rebuild", 995), ("sudo", "systemctl", 980),
            ("systemctl", "status", 995), ("systemctl", "restart", 985), ("systemctl", "start", 975),
            ("hyprland", "config", 990), ("hyprosk", "daemon", 990),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heliboard_binary_dictionary_loading() {
        let dict = Dictionary::new_empty();
        assert_eq!(dict.unigrams.len(), 55000);
        assert!(dict.bigrams.len() >= 100_000);

        // Test prefix completion on HeliBoard corpus
        let comps = dict.find_completions("cant", 4);
        assert!(!comps.is_empty());
        assert_eq!(comps[0].0, "can't");

        let comps = dict.find_completions("dont", 4);
        assert!(!comps.is_empty());
        assert_eq!(comps[0].0, "don't");

        let comps = dict.find_completions("omw", 4);
        assert!(!comps.is_empty());
        assert_eq!(comps[0].0, "on my way");

        // Test bigram prediction
        let next = dict.predict_next_words("how", 3);
        assert!(!next.is_empty());
        assert!(next.iter().any(|(w, _)| w == "are" || w == "do" || w == "many" || w == "is"));
    }
}
