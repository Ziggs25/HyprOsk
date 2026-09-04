use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"HYPROSK\0";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone)]
struct RawWord {
    word: String,
    freq: u16,
    bigrams: Vec<(String, u16)>,
}

/// Compiles an AOSP / HeliBoard `.combined` wordlist into a compact,
/// zero-copy binary `.hyprosk.dict` file.
///
/// # Arguments
/// * `input_path` - Path to the `.combined` text file.
/// * `output_path` - Path to write the output `.hyprosk.dict` binary file.
/// * `max_words` - Maximum number of top-frequency unigrams to include (e.g. 50,000).
/// * `max_bigrams_per_word` - Maximum number of bigrams to keep per word (e.g. 3).
pub fn compile_combined<P: AsRef<Path>, Q: AsRef<Path>>(
    input_path: P,
    output_path: Q,
    max_words: usize,
    max_bigrams_per_word: usize,
) -> anyhow::Result<(usize, usize, usize)> {
    let file = File::open(input_path.as_ref())?;
    let reader = BufReader::with_capacity(512 * 1024, file);

    let mut words_map: HashMap<String, RawWord> = HashMap::with_capacity(180_000);
    let mut current_word: Option<String> = None;

    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim_end();

        if let Some(rest) = trimmed.strip_prefix(" word=") {
            let mut parts = rest.split(',');
            if let Some(w) = parts.next() {
                let clean_word = w.trim().to_lowercase();
                if clean_word.is_empty() || (clean_word.len() == 1 && clean_word != "a" && clean_word != "i") {
                    current_word = None;
                    continue;
                }
                // Skip words with numbers or strange symbols, but allow apostrophes (' for can't, don't, etc.)
                if !clean_word.chars().all(|c| c.is_ascii_alphabetic() || c == '\'') {
                    current_word = None;
                    continue;
                }

                let mut freq: u16 = 100;
                for part in parts {
                    if let Some(f_str) = part.strip_prefix("f=") {
                        if let Ok(val) = f_str.parse::<u16>() {
                            // Scale 0..255 to 100..10000 range
                            freq = (val as u32 * 39 + 55) as u16;
                        }
                    }
                }

                current_word = Some(clean_word.clone());
                words_map.entry(clean_word.clone()).or_insert_with(|| RawWord {
                    word: clean_word,
                    freq,
                    bigrams: Vec::new(),
                });
            }
        } else if let Some(rest) = trimmed.strip_prefix("  bigram=") {
            if let Some(ref parent) = current_word {
                let mut parts = rest.split(',');
                if let Some(target) = parts.next() {
                    let clean_target = target.trim().to_lowercase();
                    if !clean_target.is_empty() && clean_target.chars().all(|c| c.is_ascii_alphabetic() || c == '\'') {
                        let mut b_freq: u16 = 100;
                        for part in parts {
                            if let Some(f_str) = part.strip_prefix("f=") {
                                if let Ok(val) = f_str.parse::<u16>() {
                                    // AOSP bigram freq is 1, 2, 3 or 0..255
                                    b_freq = match val {
                                        1 => 500,
                                        2 => 750,
                                        3 => 1000,
                                        other => (other as u32 * 4) as u16,
                                    };
                                }
                            }
                        }
                        if let Some(rw) = words_map.get_mut(parent) {
                            rw.bigrams.push((clean_target, b_freq));
                        }
                    }
                }
            }
        }
    }

    // Always ensure common modern/Linux terms are present with high rank
    let extra_curated: &[(&str, u16)] = &[
        ("hyprland", 4500), ("hyprosk", 5500), ("wayland", 4200), ("nixos", 4800),
        ("linux", 4400), ("rust", 4300), ("cargo", 4100), ("github", 4000),
        ("terminal", 3800), ("heliboard", 4200), ("keyboard", 3900), ("thanks", 5000),
        ("thank", 4800), ("welcome", 4200), ("please", 4500), ("sure", 4200),
        ("awesome", 4000), ("bro", 5200), ("doing", 4400), ("today", 4300),
    ];
    for &(w, f) in extra_curated {
        words_map.entry(w.to_string())
            .and_modify(|rw| { if rw.freq < f { rw.freq = f; } })
            .or_insert_with(|| RawWord {
                word: w.to_string(),
                freq: f,
                bigrams: Vec::new(),
            });
    }

    // Curated high-priority speech and command bigram chains
    let curated_bigrams: &[(&str, &str, u16)] = &[
        ("how", "are", 1000), ("how", "is", 950), ("how", "can", 900), ("how", "to", 850),
        ("are", "you", 1000), ("are", "we", 900),
        ("you", "doing", 1000), ("you", "ready", 950), ("you", "know", 900),
        ("doing", "today", 1000), ("doing", "well", 950), ("doing", "fine", 900),
        ("thank", "you", 1000), ("thanks", "bro", 1000), ("thanks", "for", 950),
        ("what", "is", 1000), ("what", "are", 950), ("where", "is", 1000),
        ("nixos", "rebuild", 1000), ("rebuild", "switch", 1000),
    ];
    for &(w1, w2, f) in curated_bigrams {
        if let Some(rw) = words_map.get_mut(w1) {
            rw.bigrams.insert(0, (w2.to_string(), f));
        }
    }

    // Sort all words by frequency descending to pick the top `max_words`
    let mut word_list: Vec<RawWord> = words_map.into_values().collect();
    word_list.sort_by(|a, b| b.freq.cmp(&a.freq));
    word_list.truncate(max_words);

    // Sort selected words alphabetically (critical for binary search on prefix scanning)
    word_list.sort_by(|a, b| a.word.cmp(&b.word));

    let word_count = word_list.len();
    let mut word_to_idx: HashMap<String, u32> = HashMap::with_capacity(word_count);
    for (idx, rw) in word_list.iter().enumerate() {
        word_to_idx.insert(rw.word.clone(), idx as u32);
    }

    // Build string pool & unigrams
    let mut string_blob = String::with_capacity(word_count * 7);
    let mut unigram_records: Vec<(u32, u16, u16)> = Vec::with_capacity(word_count);

    for rw in &word_list {
        let offset = string_blob.len() as u32;
        let len = rw.word.len() as u16;
        string_blob.push_str(&rw.word);
        unigram_records.push((offset, len, rw.freq));
    }

    // Build bigram records
    let mut bigram_records: Vec<(u32, u32, u16)> = Vec::with_capacity(word_count * 2);
    for (src_idx, rw) in word_list.iter().enumerate() {
        let mut count = 0;
        for (dst_word, freq) in &rw.bigrams {
            if let Some(&dst_idx) = word_to_idx.get(dst_word) {
                bigram_records.push((src_idx as u32, dst_idx, *freq));
                count += 1;
                if count >= max_bigrams_per_word {
                    break;
                }
            }
        }
    }
    // Sort bigrams primarily by src_idx for binary search
    bigram_records.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.2.cmp(&a.2)));

    let bigram_count = bigram_records.len();
    let string_blob_bytes = string_blob.as_bytes();
    let string_pool_len = string_blob_bytes.len() as u32;

    // Write binary file
    let out_file = File::create(output_path.as_ref())?;
    let mut writer = BufWriter::with_capacity(256 * 1024, out_file);

    // 1. Header (32 bytes)
    writer.write_all(MAGIC)?; // 8 bytes
    writer.write_all(&FORMAT_VERSION.to_le_bytes())?; // 4 bytes
    writer.write_all(&(word_count as u32).to_le_bytes())?; // 4 bytes
    writer.write_all(&(bigram_count as u32).to_le_bytes())?; // 4 bytes
    writer.write_all(&string_pool_len.to_le_bytes())?; // 4 bytes
    writer.write_all(&[0u8; 8])?; // 8 bytes padding to 32 bytes

    // 2. String Pool
    writer.write_all(string_blob_bytes)?;

    // 3. Unigram Table (word_count * 8 bytes)
    for (offset, len, freq) in unigram_records {
        writer.write_all(&offset.to_le_bytes())?;
        writer.write_all(&len.to_le_bytes())?;
        writer.write_all(&freq.to_le_bytes())?;
    }

    // 4. Bigram Table (bigram_count * 10 bytes)
    for (src_idx, dst_idx, freq) in bigram_records {
        writer.write_all(&src_idx.to_le_bytes())?;
        writer.write_all(&dst_idx.to_le_bytes())?;
        writer.write_all(&freq.to_le_bytes())?;
    }

    writer.flush()?;

    let total_file_size = 32 + (string_pool_len as usize) + (word_count * 8) + (bigram_count * 10);
    Ok((word_count, bigram_count, total_file_size))
}
