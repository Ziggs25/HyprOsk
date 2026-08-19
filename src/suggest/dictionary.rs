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

    fn load_default_words(&mut self) {
        // High frequency English & Linux/Wayland developer words
        let word_list = [
            ("the", 100000), ("be", 90000), ("to", 85000), ("of", 80000), ("and", 75000),
            ("a", 70000), ("in", 65000), ("that", 60000), ("have", 55000), ("i", 50000),
            ("it", 48000), ("for", 46000), ("not", 44000), ("on", 42000), ("with", 40000),
            ("he", 38000), ("as", 36000), ("you", 34000), ("do", 32000), ("at", 30000),
            ("this", 29000), ("but", 28000), ("his", 27000), ("by", 26000), ("from", 25000),
            ("they", 24000), ("we", 23000), ("say", 22000), ("her", 21000), ("she", 20000),
            ("or", 19000), ("an", 18000), ("will", 17000), ("my", 16000), ("one", 15000),
            ("all", 14000), ("would", 13000), ("there", 12000), ("their", 11000), ("what", 10000),
            ("so", 9800), ("up", 9600), ("out", 9400), ("if", 9200), ("about", 9000),
            ("who", 8800), ("get", 8600), ("which", 8400), ("go", 8200), ("me", 8000),
            ("when", 7800), ("make", 7600), ("can", 7400), ("like", 7200), ("time", 7000),
            ("no", 6800), ("just", 6600), ("him", 6400), ("know", 6200), ("take", 6000),
            ("people", 5800), ("into", 5600), ("year", 5400), ("your", 5200), ("good", 5000),
            ("some", 4800), ("could", 4600), ("them", 4400), ("see", 4200), ("other", 4000),
            ("than", 3800), ("then", 3600), ("now", 3400), ("look", 3200), ("only", 3000),
            ("come", 2800), ("its", 2600), ("over", 2400), ("think", 2200), ("also", 2000),
            ("back", 1900), ("after", 1800), ("use", 1700), ("two", 1600), ("how", 1500),
            ("our", 1400), ("work", 1300), ("first", 1200), ("well", 1100), ("way", 1000),
            ("even", 950), ("new", 900), ("want", 850), ("because", 800), ("any", 750),
            ("these", 700), ("give", 650), ("day", 600), ("most", 550), ("us", 500),
            ("hello", 4500), ("help", 4000), ("here", 3800), ("hope", 3500), ("home", 3200),
            ("hyprland", 5000), ("wayland", 4800), ("linux", 4700), ("rust", 4600), ("cargo", 4500),
            ("terminal", 4000), ("browser", 3900), ("keyboard", 4200), ("heliboard", 4000), ("hyprosk", 5000),
            ("awesome", 3000), ("great", 3500), ("thanks", 4200), ("thank", 4100), ("please", 3900),
            ("sorry", 2500), ("sure", 2800), ("today", 3100), ("tomorrow", 3000), ("tonight", 2900),
            ("google", 3500), ("github", 3600), ("commit", 3400), ("push", 3300), ("pull", 3200),
            ("update", 3100), ("config", 3500), ("system", 3000), ("window", 2800), ("screen", 2900),
        ];

        for (word, freq) in word_list {
            self.insert(word, freq);
        }
    }
}
