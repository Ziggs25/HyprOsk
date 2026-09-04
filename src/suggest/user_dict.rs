use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_WORD_LEN: usize = 32;
pub const MAX_USER_WORDS: usize = 15_000;
pub const MAX_USER_BIGRAMS: usize = 5_000;

/// Static contraction and acronym shortcut expansion table with smart alternatives
pub static CONTRACTIONS: &[(&str, &str, &[&str])] = &[
    ("cant", "can't", &["cannot", "can"]),
    ("dont", "don't", &["done", "do"]),
    ("wont", "won't", &["want", "went"]),
    ("im", "I'm", &["in", "is"]),
    ("id", "I'd", &["idea", "if"]),
    ("ill", "I'll", &["will", "all"]),
    ("ive", "I've", &["give", "ice"]),
    ("youre", "you're", &["your", "you"]),
    ("youll", "you'll", &["you", "your"]),
    ("youve", "you've", &["you", "your"]),
    ("theyre", "they're", &["their", "there"]),
    ("theyll", "they'll", &["they", "them"]),
    ("theyve", "they've", &["they", "their"]),
    ("isnt", "isn't", &["is", "issue"]),
    ("arent", "aren't", &["are", "area"]),
    ("didnt", "didn't", &["did", "done"]),
    ("doesnt", "doesn't", &["does", "doing"]),
    ("havent", "haven't", &["have", "haven"]),
    ("hasnt", "hasn't", &["has", "had"]),
    ("hadnt", "hadn't", &["had", "have"]),
    ("wasnt", "wasn't", &["was", "waste"]),
    ("werent", "weren't", &["were", "where"]),
    ("couldnt", "couldn't", &["could", "would"]),
    ("wouldnt", "wouldn't", &["would", "could"]),
    ("shouldnt", "shouldn't", &["should", "show"]),
    ("thats", "that's", &["that", "thanks"]),
    ("whats", "what's", &["what", "whatever"]),
    ("hows", "how's", &["how", "however"]),
    ("heres", "here's", &["here", "help"]),
    ("theres", "there's", &["there", "their"]),
    ("lets", "let's", &["let", "letter"]),
    ("omw", "on my way", &["omg", "now"]),
    ("brb", "be right back", &["bro", "back"]),
    ("tbh", "to be honest", &["the", "to"]),
    ("idk", "I don't know", &["idea", "i"]),
    ("imo", "in my opinion", &["in", "into"]),
    ("fyi", "for your information", &["for", "first"]),
    ("btw", "by the way", &["by", "between"]),
];

#[inline]
pub fn find_shortcut(prefix: &str) -> Option<&'static str> {
    for &(abbr, full, _) in CONTRACTIONS {
        if prefix.eq_ignore_ascii_case(abbr) {
            return Some(full);
        }
    }
    None
}

#[inline]
pub fn find_shortcut_details(prefix: &str) -> Option<(&'static str, &'static [&'static str])> {
    for &(abbr, full, alts) in CONTRACTIONS {
        if prefix.eq_ignore_ascii_case(abbr) {
            return Some((full, alts));
        }
    }
    None
}

/// Bit-packed User History Word Entry (38 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserWordEntry {
    pub word: [u8; MAX_WORD_LEN],
    pub len: u8,
    pub freq: u16,
    pub count: u16,
    pub last_used_epoch: u16,
    pub is_permanent: u8, // 1 = Permanent / Pinned (immune to eviction), 0 = Transient
}

impl UserWordEntry {
    #[inline]
    pub fn as_str(&self) -> &str {
        let valid = &self.word[..self.len as usize];
        std::str::from_utf8(valid).unwrap_or("")
    }
}

/// Bit-packed User History Bigram Entry (68 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserBigramEntry {
    pub prev_word: [u8; MAX_WORD_LEN],
    pub next_word: [u8; MAX_WORD_LEN],
    pub prev_len: u8,
    pub next_len: u8,
    pub count: u16,
}

impl UserBigramEntry {
    #[inline]
    pub fn prev_str(&self) -> &str {
        let valid = &self.prev_word[..self.prev_len as usize];
        std::str::from_utf8(valid).unwrap_or("")
    }

    #[inline]
    pub fn next_str(&self) -> &str {
        let valid = &self.next_word[..self.next_len as usize];
        std::str::from_utf8(valid).unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct UserDictionary {
    pub words: Vec<UserWordEntry>,
    pub bigrams: Vec<UserBigramEntry>,
    pub current_epoch: u16,
    pub is_dirty: bool,
    pub save_path: Option<PathBuf>,
}

impl Default for UserDictionary {
    fn default() -> Self {
        Self::new()
    }
}

impl UserDictionary {
    pub fn new() -> Self {
        let default_path = Self::default_storage_path();
        let mut dict = Self::new_empty();
        dict.save_path = default_path.clone();

        if let Some(ref path) = default_path {
            let _ = dict.load_from_disk(path);
        }

        dict
    }

    /// Creates an empty UserDictionary without loading from disk (useful for tests or custom paths).
    pub fn new_empty() -> Self {
        Self {
            words: Vec::with_capacity(128),
            bigrams: Vec::with_capacity(64),
            current_epoch: 0,
            is_dirty: false,
            save_path: None,
        }
    }

    pub fn default_storage_path() -> Option<PathBuf> {
        let base = if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            PathBuf::from(xdg)
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".local/share")
        } else {
            return None;
        };
        Some(base.join("hyprosk/user_dict.bin"))
    }

    /// Records a typed word.
    ///
    /// - `is_explicit_selection`: Set to true when tapped from the suggestion bar.
    /// - If typed 2+ times or explicitly selected, marked as `is_permanent = 1` (NEVER deleted).
    pub fn record_word(&mut self, raw_word: &str, is_explicit_selection: bool) {
        let clean = raw_word.trim().to_lowercase();
        if clean.len() < 2 || clean.len() > MAX_WORD_LEN {
            if clean != "a" && clean != "i" {
                return;
            }
        }
        let bytes = clean.as_bytes();
        let len = bytes.len() as u8;

        self.current_epoch = self.current_epoch.wrapping_add(1);

        // 1. Search for existing entry
        for entry in &mut self.words {
            if entry.len == len && &entry.word[..len as usize] == bytes {
                entry.count = entry.count.saturating_add(1);
                entry.freq = entry.freq.saturating_add(15).min(255);
                entry.last_used_epoch = self.current_epoch;
                // Once reinforced or explicitly selected, permanently immune to pruning
                if entry.count >= 2 || is_explicit_selection {
                    entry.is_permanent = 1;
                }
                self.is_dirty = true;
                return;
            }
        }

        // 2. Capacity check
        if self.words.len() >= MAX_USER_WORDS {
            self.decay_and_prune();
        }

        // 3. Insert new word entry
        let mut w = [0u8; MAX_WORD_LEN];
        w[..len as usize].copy_from_slice(bytes);

        self.words.push(UserWordEntry {
            word: w,
            len,
            freq: 60,
            count: 1,
            last_used_epoch: self.current_epoch,
            is_permanent: if is_explicit_selection { 1 } else { 0 },
        });
        self.is_dirty = true;
    }

    /// Records a bigram transition (prev_word -> next_word).
    pub fn record_bigram(&mut self, prev: &str, next: &str) {
        let p_clean = prev.trim().to_lowercase();
        let n_clean = next.trim().to_lowercase();
        if p_clean.is_empty() || n_clean.is_empty() || p_clean == n_clean {
            return;
        }

        let p_bytes = p_clean.as_bytes();
        let n_bytes = n_clean.as_bytes();
        let p_len = p_bytes.len().min(MAX_WORD_LEN) as u8;
        let n_len = n_bytes.len().min(MAX_WORD_LEN) as u8;

        for b in &mut self.bigrams {
            if b.prev_len == p_len && b.next_len == n_len
                && &b.prev_word[..p_len as usize] == &p_bytes[..p_len as usize]
                && &b.next_word[..n_len as usize] == &n_bytes[..n_len as usize]
            {
                b.count = b.count.saturating_add(1);
                self.is_dirty = true;
                return;
            }
        }

        if self.bigrams.len() >= MAX_USER_BIGRAMS {
            // Prune lowest count bigrams
            if let Some(min_idx) = self.bigrams.iter().enumerate().min_by_key(|(_, b)| b.count).map(|(i, _)| i) {
                self.bigrams.swap_remove(min_idx);
            }
        }

        let mut pw = [0u8; MAX_WORD_LEN];
        let mut nw = [0u8; MAX_WORD_LEN];
        pw[..p_len as usize].copy_from_slice(&p_bytes[..p_len as usize]);
        nw[..n_len as usize].copy_from_slice(&n_bytes[..n_len as usize]);

        self.bigrams.push(UserBigramEntry {
            prev_word: pw,
            next_word: nw,
            prev_len: p_len,
            next_len: n_len,
            count: 1,
        });
        self.is_dirty = true;
    }

    /// Reverts a newly added word if user immediately backspaces across the space.
    pub fn revert_last_word(&mut self, word: &str) {
        let clean = word.trim().to_lowercase();
        let bytes = clean.as_bytes();
        let len = bytes.len() as u8;

        if let Some(pos) = self.words.iter().position(|e| e.len == len && &e.word[..len as usize] == bytes) {
            if self.words[pos].is_permanent == 0 && self.words[pos].count <= 1 {
                self.words.swap_remove(pos);
                self.is_dirty = true;
            } else {
                self.words[pos].count = self.words[pos].count.saturating_sub(1);
                self.words[pos].freq = self.words[pos].freq.saturating_sub(15);
                self.is_dirty = true;
            }
        }
    }

    /// Scans user words for completions matching prefix.
    /// Appends matching (word, score) candidates.
    pub fn find_completions<'a>(&'a self, prefix: &str, out: &mut Vec<(&'a str, u32)>) {
        if prefix.is_empty() {
            return;
        }
        let prefix_lower = prefix.to_lowercase();
        let prefix_bytes = prefix_lower.as_bytes();
        let p_len = prefix_bytes.len();

        for entry in &self.words {
            if (entry.len as usize) >= p_len && &entry.word[..p_len] == prefix_bytes {
                let w = entry.as_str();
                if !w.is_empty() {
                    // User history words get high priority boost + frequency weight
                    let score = 30_000u32 + (entry.freq as u32 * 15) + ((entry.is_permanent as u32) * 5_000);
                    out.push((w, score));
                }
            }
        }
    }

    /// Predicts next words from learned bigrams.
    pub fn predict_next<'a>(&'a self, prev_word: &str, out: &mut Vec<(&'a str, u32)>) {
        let prev_lower = prev_word.trim().to_lowercase();
        if prev_lower.is_empty() {
            return;
        }
        let p_bytes = prev_lower.as_bytes();
        let p_len = p_bytes.len() as u8;

        for b in &self.bigrams {
            if b.prev_len == p_len && &b.prev_word[..p_len as usize] == p_bytes {
                let n = b.next_str();
                if !n.is_empty() {
                    let score = 50_000u32 + (b.count as u32 * 200);
                    out.push((n, score));
                }
            }
        }
    }

    /// Prunes transient typos when approaching capacity.
    /// PERMANENT WORDS ARE NEVER DELETED.
    pub fn decay_and_prune(&mut self) {
        let epoch = self.current_epoch;
        self.words.retain_mut(|entry| {
            if entry.is_permanent == 1 {
                return true; // NEVER delete permanent words
            }
            // Multiplicative decay for transient entries
            entry.freq = (entry.freq as u32 * 8 / 10) as u16;
            let age = epoch.saturating_sub(entry.last_used_epoch);
            // Prune unreinforced typos with low frequency and older than 500 epochs
            entry.freq >= 15 && age < 500
        });
        self.is_dirty = true;
    }

    /// Saves the user dictionary to disk atomically.
    pub fn save_to_disk(&mut self, path: &Path) -> std::io::Result<()> {
        if !self.is_dirty {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("tmp");
        {
            let file = File::create(&tmp_path)?;
            let mut writer = BufWriter::new(file);

            // Magic Header: "HYP2"
            writer.write_all(b"HYP2")?;
            writer.write_all(&(self.words.len() as u32).to_le_bytes())?;
            writer.write_all(&(self.bigrams.len() as u32).to_le_bytes())?;
            writer.write_all(&self.current_epoch.to_le_bytes())?;

            for e in &self.words {
                writer.write_all(&e.word)?;
                writer.write_all(&[e.len])?;
                writer.write_all(&e.freq.to_le_bytes())?;
                writer.write_all(&e.count.to_le_bytes())?;
                writer.write_all(&e.last_used_epoch.to_le_bytes())?;
                writer.write_all(&[e.is_permanent])?;
            }

            for b in &self.bigrams {
                writer.write_all(&b.prev_word)?;
                writer.write_all(&b.next_word)?;
                writer.write_all(&[b.prev_len, b.next_len])?;
                writer.write_all(&b.count.to_le_bytes())?;
            }

            writer.flush()?;
        }

        fs::rename(tmp_path, path)?;
        self.is_dirty = false;
        Ok(())
    }

    /// Loads the user dictionary from disk.
    pub fn load_from_disk(&mut self, path: &Path) -> std::io::Result<()> {
        if !path.exists() {
            return Ok(());
        }

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != b"HYP2" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid user dictionary format",
            ));
        }

        let mut u32_buf = [0u8; 4];
        reader.read_exact(&mut u32_buf)?;
        let word_count = u32::from_le_bytes(u32_buf) as usize;

        reader.read_exact(&mut u32_buf)?;
        let bigram_count = u32::from_le_bytes(u32_buf) as usize;

        let mut u16_buf = [0u8; 2];
        reader.read_exact(&mut u16_buf)?;
        self.current_epoch = u16::from_le_bytes(u16_buf);

        self.words.clear();
        self.words.reserve(word_count.min(MAX_USER_WORDS));

        for _ in 0..word_count.min(MAX_USER_WORDS) {
            let mut word = [0u8; MAX_WORD_LEN];
            reader.read_exact(&mut word)?;
            let mut len_buf = [0u8; 1];
            reader.read_exact(&mut len_buf)?;
            let len = len_buf[0];

            reader.read_exact(&mut u16_buf)?;
            let freq = u16::from_le_bytes(u16_buf);

            reader.read_exact(&mut u16_buf)?;
            let count = u16::from_le_bytes(u16_buf);

            reader.read_exact(&mut u16_buf)?;
            let last_used_epoch = u16::from_le_bytes(u16_buf);

            let mut perm_buf = [0u8; 1];
            reader.read_exact(&mut perm_buf)?;
            let is_permanent = perm_buf[0];

            self.words.push(UserWordEntry {
                word,
                len,
                freq,
                count,
                last_used_epoch,
                is_permanent,
            });
        }

        self.bigrams.clear();
        self.bigrams.reserve(bigram_count.min(MAX_USER_BIGRAMS));

        for _ in 0..bigram_count.min(MAX_USER_BIGRAMS) {
            let mut prev_word = [0u8; MAX_WORD_LEN];
            let mut next_word = [0u8; MAX_WORD_LEN];
            reader.read_exact(&mut prev_word)?;
            reader.read_exact(&mut next_word)?;

            let mut lens = [0u8; 2];
            reader.read_exact(&mut lens)?;

            reader.read_exact(&mut u16_buf)?;
            let count = u16::from_le_bytes(u16_buf);

            self.bigrams.push(UserBigramEntry {
                prev_word,
                next_word,
                prev_len: lens[0],
                next_len: lens[1],
                count,
            });
        }

        self.is_dirty = false;
        Ok(())
    }

    /// Flushes changes to disk if dirty.
    pub fn flush_if_dirty(&mut self) {
        if self.is_dirty {
            if let Some(path) = self.save_path.clone() {
                let _ = self.save_to_disk(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_permanent_retention() {
        let mut dict = UserDictionary::new_empty();
        dict.record_word("hyprosk", false);
        assert_eq!(dict.words.len(), 1);
        assert_eq!(dict.words[0].count, 1);
        assert_eq!(dict.words[0].is_permanent, 0);

        // Record again -> becomes permanent
        dict.record_word("hyprosk", false);
        assert_eq!(dict.words.len(), 1);
        assert_eq!(dict.words[0].count, 2);
        assert_eq!(dict.words[0].is_permanent, 1);

        // Pruning does not delete permanent words
        dict.current_epoch = 1000;
        dict.decay_and_prune();
        assert_eq!(dict.words.len(), 1);
        assert_eq!(dict.words[0].as_str(), "hyprosk");
    }

    #[test]
    fn test_explicit_selection_becomes_permanent() {
        let mut dict = UserDictionary::new_empty();
        dict.record_word("coolslang", true); // explicitly selected
        assert_eq!(dict.words.len(), 1);
        assert_eq!(dict.words[0].is_permanent, 1);
    }

    #[test]
    fn test_shortcut_lookup() {
        assert_eq!(find_shortcut("cant"), Some("can't"));
        assert_eq!(find_shortcut("CANT"), Some("can't"));
        assert_eq!(find_shortcut("omw"), Some("on my way"));
        assert_eq!(find_shortcut("hello"), None);
    }

    #[test]
    fn test_disk_save_and_load() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("hyprosk_test_user_dict.bin");

        let mut dict = UserDictionary::new_empty();
        dict.record_word("ziggs", true);
        dict.record_word("hyprland", false);
        dict.record_bigram("ziggs", "hyprland");

        assert!(dict.save_to_disk(&test_file).is_ok());

        let mut loaded = UserDictionary::new_empty();
        assert!(loaded.load_from_disk(&test_file).is_ok());

        assert_eq!(loaded.words.len(), 2);
        assert_eq!(loaded.words[0].as_str(), "ziggs");
        assert_eq!(loaded.words[0].is_permanent, 1);
        assert_eq!(loaded.words[1].as_str(), "hyprland");
        assert_eq!(loaded.bigrams.len(), 1);
        assert_eq!(loaded.bigrams[0].prev_str(), "ziggs");
        assert_eq!(loaded.bigrams[0].next_str(), "hyprland");

        let _ = fs::remove_file(test_file);
    }
}
