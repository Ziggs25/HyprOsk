use crate::suggest::dictionary::Dictionary;

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn dedup_path(path: &[char]) -> Vec<char> {
    let mut out = Vec::new();
    for &c in path {
        if out.last() != Some(&c) {
            out.push(c);
        }
    }
    out
}

pub fn swipe_candidates(path: &[char], dict: &Dictionary) -> Vec<String> {
    if path.len() < 2 {
        return Vec::new();
    }
    let swipe = dedup_path(path);
    if swipe.len() < 2 {
        return Vec::new();
    }
    let swipe_lower: Vec<char> = swipe.iter().map(|c| c.to_ascii_lowercase()).collect();
    let all = dict.all_words(4000);
    let mut scored: Vec<(String, i32)> = Vec::new();
    for (word, freq) in all {
        if word.len() < 2 || word.len() > swipe.len() + 6 {
            continue;
        }
        let wchars: Vec<char> = word.chars().collect();
        let dist = levenshtein(&swipe_lower, &wchars);
        if dist > 4 && !word.starts_with(swipe_lower[0]) {
            continue;
        }
        if word.chars().next() != swipe_lower.first().copied() {
            continue;
        }
        if word.chars().last() != swipe_lower.last().copied() && dist > 2 {
            continue;
        }
        let score = freq as i32 - (dist as i32 * 1800) - ((swipe.len() as i32 - word.len() as i32).abs() * 200);
        scored.push((word, score));
    }
    scored.sort_by_key(|b| std::cmp::Reverse(b.1));
    scored.into_iter().take(3).map(|(w, _)| w).collect()
}
