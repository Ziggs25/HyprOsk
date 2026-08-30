use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct TrieNode {
    pub children: HashMap<char, TrieNode>,
    pub is_word: bool,
    pub frequency: u32,
}

#[derive(Debug, Clone)]
pub struct Dictionary {
    root: TrieNode,
}

impl Default for Dictionary {
    fn default() -> Self {
        Self::new()
    }
}

impl Dictionary {
    pub fn new() -> Self {
        let mut dict = Self {
            root: TrieNode::default(),
        };
        dict.load_default_words();
        dict
    }

    pub fn insert(&mut self, word: &str, frequency: u32) {
        let mut current = &mut self.root;
        for ch in word.to_lowercase().chars() {
            current = current.children.entry(ch).or_default();
        }
        current.is_word = true;
        current.frequency = frequency;
    }

    pub fn find_completions(&self, prefix: &str, limit: usize) -> Vec<(String, u32)> {
        let prefix_lower = prefix.to_lowercase();
        let mut current = &self.root;

        for ch in prefix_lower.chars() {
            if let Some(next) = current.children.get(&ch) {
                current = next;
            } else {
                return Vec::new();
            }
        }

        let mut results = Vec::new();
        let mut path = prefix_lower.clone();
        Self::dfs(current, &mut path, &mut results);

        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.truncate(limit);
        results
    }

    fn dfs(node: &TrieNode, path: &mut String, results: &mut Vec<(String, u32)>) {
        if node.is_word {
            results.push((path.clone(), node.frequency));
        }

        for (&ch, child) in &node.children {
            path.push(ch);
            Self::dfs(child, path, results);
            path.pop();
        }
    }

    pub fn all_words(&self, limit: usize) -> Vec<(String, u32)> {
        let mut results = Vec::new();
        let mut path = String::new();
        Self::dfs(&self.root, &mut path, &mut results);
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.truncate(limit);
        results
    }

    fn load_default_words(&mut self) {
        let mut rank = 0u32;
        let max_rank = 10000u32;
        let wordlist = if let Ok(c) = std::fs::read_to_string("assets/google-10000-english.txt") { c }
            else { include_str!("../../assets/google-10000-english.txt").to_string() };
        for line in wordlist.lines() {
            let w = line.trim().to_lowercase();
            if w.is_empty() || w.len() <= 1 && !["a", "i"].contains(&w.as_str()) {
                continue;
            }
            let freq = max_rank.saturating_sub(rank).saturating_add(1000);
            self.insert(&w, freq);
            rank += 1;
            if rank >= 8000 {
                break;
            }
        }
        if self.root.children.is_empty() {
            let fallback = [
                ("the", 100000), ("be", 90000), ("to", 85000), ("of", 80000), ("and", 75000),
                ("a", 70000), ("in", 65000), ("that", 60000), ("have", 55000), ("i", 50000),
            ];
            for (word, freq) in fallback {
                self.insert(word, freq);
            }
        }
        let linux_words = [
            ("hello", 4500), ("help", 4000), ("hyprland", 5000), ("wayland", 4800), ("linux", 4700),
            ("rust", 4600), ("cargo", 4500), ("terminal", 4000), ("browser", 3900), ("keyboard", 4200),
            ("heliboard", 4000), ("hyprosk", 5000), ("thanks", 4200), ("google", 3500), ("github", 3600),
        ];
        for (word, freq) in linux_words {
            self.insert(word, freq);
        }
    }
}
