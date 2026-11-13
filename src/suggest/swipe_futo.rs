use std::collections::HashMap;
use std::path::{Path, PathBuf};

use executorch::{evalue::EValue, module::Module, tensor::TensorPtr};
use ndarray::{Array2, Array3, Ix3};

use crate::suggest::dictionary::Dictionary;

const MAX_KEYS: usize = 64;
const INPUT_STEPS: usize = 64;
const OUTPUT_STEPS: usize = 32;
const BLANK_CLASS: usize = 64;
const BEAM_WIDTH: usize = 100;
const FREQUENCY_BONUS: f32 = 0.1017;
const MAX_WORD_LEN: usize = 32;

/// A letter key's center and half-size in normalized letter-area coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwipeKeyGeom {
    pub letter: char,
    pub cx: f32,
    pub cy: f32,
    pub hw: f32,
    pub hh: f32,
}

#[derive(Debug, Clone, Copy)]
enum WordRef {
    Base(usize),
    User(usize),
}

#[derive(Clone, Copy)]
struct BeamScore {
    blank: f32,
    nonblank: f32,
}

impl BeamScore {
    fn empty() -> Self {
        Self {
            blank: f32::NEG_INFINITY,
            nonblank: f32::NEG_INFINITY,
        }
    }

    fn total(self) -> f32 {
        log_add(self.blank, self.nonblank)
    }
}

/// FUTO's layout-agnostic encoder plus a Rust lexicon-constrained CTC decoder.
/// The lexicon is a sorted index of references into HyprOsk's existing base
/// dictionary and user dictionary; it does not copy words into a C++ trie.
pub struct SwipeRecognizer {
    module: Module<'static>,
    letters: Vec<char>,
    geometry: Vec<SwipeKeyGeom>,
    lexicon: Vec<WordRef>,
    lexicon_revision: Option<u64>,
    warned_inference: bool,
}

impl SwipeRecognizer {
    pub fn new() -> Option<Self> {
        let path = model_path()?;
        Some(Self {
            module: Module::new(path),
            letters: Vec::new(),
            geometry: Vec::new(),
            lexicon: Vec::new(),
            lexicon_revision: None,
            warned_inference: false,
        })
    }

    /// Stores the active layout in sorted letter order, as expected by FUTO's
    /// character classes. Returns true when its letters or key centers changed.
    pub fn prepare_layout(&mut self, keys: &[SwipeKeyGeom]) -> bool {
        let mut sorted: Vec<_> = keys
            .iter()
            .copied()
            .filter(|key| key.letter.is_ascii_alphabetic())
            .map(|mut key| {
                key.letter = key.letter.to_ascii_lowercase();
                key
            })
            .collect();
        sorted.sort_by_key(|key| key.letter);
        sorted.dedup_by_key(|key| key.letter);
        sorted.truncate(MAX_KEYS);
        if sorted.len() < 2 {
            return false;
        }

        let letters: Vec<char> = sorted.iter().map(|key| key.letter).collect();
        let changed = letters != self.letters || sorted != self.geometry;
        self.letters = letters;
        self.geometry = sorted;
        changed
    }

    /// Runs the FUTO encoder and CTC beam search against HyprOsk's current
    /// dictionary. `points` use normalized x/y and milliseconds from gesture start.
    pub fn decode(
        &mut self,
        points: &[(f32, f32, f64)],
        dictionary: &Dictionary,
        _aspect: f32,
        top_n: usize,
    ) -> Vec<String> {
        if points.len() < 2 || self.letters.is_empty() || top_n == 0 {
            return Vec::new();
        }
        self.refresh_lexicon(dictionary);
        if self.lexicon.is_empty() {
            return Vec::new();
        }

        let Some(features) = resample_path(points) else {
            return Vec::new();
        };
        let mut key_centers = Array3::<f32>::zeros((1, MAX_KEYS, 2));
        let mut key_mask = Array2::<bool>::from_elem((1, MAX_KEYS), false);
        for (index, key) in self.geometry.iter().enumerate() {
            key_centers[[0, index, 0]] = key.cx;
            key_centers[[0, index, 1]] = key.cy;
            key_mask[[0, index]] = true;
        }

        let Ok(features) = TensorPtr::from_array(features) else {
            return Vec::new();
        };
        let Ok(key_centers) = TensorPtr::from_array(key_centers) else {
            return Vec::new();
        };
        let Ok(key_mask) = TensorPtr::from_array(key_mask) else {
            return Vec::new();
        };
        let inputs = [
            EValue::new(&features),
            EValue::new(&key_centers),
            EValue::new(&key_mask),
        ];
        let outputs = match self.module.forward(&inputs) {
            Ok(outputs) => outputs,
            Err(error) => {
                if !self.warned_inference {
                    tracing::warn!("FUTO Swipe inference failed: {error}");
                    self.warned_inference = true;
                }
                return Vec::new();
            }
        };
        if outputs.is_empty() {
            return Vec::new();
        }
        let frames: Vec<[f32; MAX_KEYS + 1]> = {
            let emissions = outputs[0].as_tensor().into_typed::<f32>();
            let emissions = emissions.as_array::<Ix3>();
            if emissions.shape() != [1, OUTPUT_STEPS, MAX_KEYS + 1] {
                tracing::warn!(
                    "FUTO Swipe returned an unexpected emission shape: {:?}",
                    emissions.shape()
                );
                return Vec::new();
            }
            (0..OUTPUT_STEPS)
                .map(|time| std::array::from_fn(|class| emissions[[0, time, class]]))
                .collect()
        };
        drop(outputs);
        self.decode_ctc(&frames, dictionary, top_n)
    }

    fn refresh_lexicon(&mut self, dictionary: &Dictionary) {
        let revision = dictionary.swipe_lexicon_revision();
        if self.lexicon_revision == Some(revision) {
            return;
        }

        let mut words = Vec::with_capacity(
            dictionary.swipe_base_word_count() + dictionary.user_dict.words.len(),
        );
        for index in 0..dictionary.swipe_base_word_count() {
            if let Some((word, _)) = dictionary.swipe_base_word(index)
                && valid_swipe_word(word)
            {
                words.push(WordRef::Base(index));
            }
        }
        for (index, entry) in dictionary.user_dict.words.iter().enumerate() {
            if valid_swipe_word(entry.as_str()) {
                words.push(WordRef::User(index));
            }
        }
        words.sort_unstable_by(|left, right| {
            word_for(dictionary, *left).cmp(word_for(dictionary, *right))
        });
        self.lexicon = words;
        self.lexicon_revision = Some(revision);
    }

    fn decode_ctc(
        &self,
        frames: &[[f32; MAX_KEYS + 1]],
        dictionary: &Dictionary,
        top_n: usize,
    ) -> Vec<String> {
        let mut beams = HashMap::with_capacity(BEAM_WIDTH);
        let mut initial = BeamScore::empty();
        initial.blank = 0.0;
        beams.insert(String::new(), initial);

        for frame in frames {
            let mut next = HashMap::with_capacity(BEAM_WIDTH * 4);
            for (prefix, score) in beams {
                let total = score.total();
                let blank_probability = frame[BLANK_CLASS];
                if blank_probability.is_finite() {
                    let target = next.entry(prefix.clone()).or_insert_with(BeamScore::empty);
                    target.blank = log_add(target.blank, total + blank_probability);
                }

                let last = prefix.as_bytes().last().copied();
                for (class, letter) in self.letters.iter().copied().enumerate() {
                    let probability = frame[class];
                    if !probability.is_finite() {
                        continue;
                    }
                    if last == Some(letter as u8) {
                        let target = next.entry(prefix.clone()).or_insert_with(BeamScore::empty);
                        target.nonblank = log_add(target.nonblank, score.nonblank + probability);
                    }

                    let mut extended = String::with_capacity(prefix.len() + 1);
                    extended.push_str(&prefix);
                    extended.push(letter);
                    if !self.has_prefix(dictionary, &extended) {
                        continue;
                    }
                    let source = if last == Some(letter as u8) {
                        score.blank
                    } else {
                        total
                    };
                    if !source.is_finite() {
                        continue;
                    }
                    let target = next.entry(extended).or_insert_with(BeamScore::empty);
                    target.nonblank = log_add(target.nonblank, source + probability);
                }
            }

            let mut ranked: Vec<_> = next.into_iter().collect();
            ranked.sort_unstable_by(|(left_word, left), (right_word, right)| {
                right
                    .total()
                    .total_cmp(&left.total())
                    .then_with(|| left_word.cmp(right_word))
            });
            ranked.truncate(BEAM_WIDTH);
            beams = ranked.into_iter().collect();
            if beams.is_empty() {
                return Vec::new();
            }
        }

        let mut candidates: Vec<(String, f32)> = beams
            .into_iter()
            .filter_map(|(word, score)| {
                let frequency = self.word_frequency(dictionary, &word)?;
                let prior = FREQUENCY_BONUS * (frequency.max(1) as f32).ln();
                Some((word, score.total() + prior))
            })
            .collect();
        candidates.sort_unstable_by(|(left_word, left), (right_word, right)| {
            right
                .total_cmp(left)
                .then_with(|| left_word.cmp(right_word))
        });
        candidates
            .into_iter()
            .take(top_n)
            .map(|(word, _)| word)
            .collect()
    }

    fn has_prefix(&self, dictionary: &Dictionary, prefix: &str) -> bool {
        let index = self
            .lexicon
            .partition_point(|entry| word_for(dictionary, *entry) < prefix);
        self.lexicon
            .get(index)
            .is_some_and(|entry| word_for(dictionary, *entry).starts_with(prefix))
    }

    fn word_frequency(&self, dictionary: &Dictionary, word: &str) -> Option<u32> {
        let mut index = self
            .lexicon
            .partition_point(|entry| word_for(dictionary, *entry) < word);
        let mut best: Option<u32> = None;
        while let Some(entry) = self.lexicon.get(index).copied() {
            let candidate = word_for(dictionary, entry);
            if candidate != word {
                break;
            }
            let frequency = match entry {
                WordRef::Base(base_index) => dictionary
                    .swipe_base_word(base_index)
                    .map(|(_, frequency)| frequency)?,
                WordRef::User(user_index) => {
                    let user = dictionary.user_dict.words.get(user_index)?;
                    u32::from(user.freq) * 32 + if user.is_permanent != 0 { 1_500 } else { 0 }
                }
            };
            best = Some(best.unwrap_or(0).max(frequency));
            index += 1;
        }
        best
    }
}

fn valid_swipe_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= MAX_WORD_LEN
        && word.bytes().all(|byte| byte.is_ascii_lowercase())
}

fn word_for(dictionary: &Dictionary, entry: WordRef) -> &str {
    match entry {
        WordRef::Base(index) => dictionary
            .swipe_base_word(index)
            .map_or("", |(word, _)| word),
        WordRef::User(index) => dictionary
            .user_dict
            .words
            .get(index)
            .map_or("", |entry| entry.as_str()),
    }
}

fn model_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HYPROSK_FUTO_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let installed = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent()?.parent().map(Path::to_path_buf))
        .map(|prefix| prefix.join("share/hyprosk/futo_swipe/model_int8.pte"));
    if let Some(path) = installed.filter(|path| path.is_file()) {
        return Some(path);
    }

    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/futo_swipe/model_int8.pte");
    source.is_file().then_some(source)
}

fn resample_path(points: &[(f32, f32, f64)]) -> Option<Array3<f32>> {
    let mut clean = Vec::with_capacity(points.len());
    let mut last_time = f64::NEG_INFINITY;
    for &(x, y, time) in points {
        if !x.is_finite() || !y.is_finite() || !time.is_finite() {
            continue;
        }
        let time = time.max(last_time);
        clean.push((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), time));
        last_time = time;
    }
    if clean.len() < 2 {
        return None;
    }

    let start = clean.first()?.2;
    let end = clean.last()?.2;
    let duration = end - start;
    let mut at_60hz = Vec::new();
    if duration > 1.0e-3 {
        let count = ((duration / (1000.0 / 60.0)).round() as usize + 1).clamp(2, 512);
        at_60hz.reserve(count);
        let mut right = 1;
        for index in 0..count {
            let time = start + duration * index as f64 / (count - 1) as f64;
            while right + 1 < clean.len() && clean[right].2 < time {
                right += 1;
            }
            let left = right.saturating_sub(1);
            let (x0, y0, t0) = clean[left];
            let (x1, y1, t1) = clean[right];
            let fraction = if t1 > t0 {
                ((time - t0) / (t1 - t0)).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            at_60hz.push((x0 + (x1 - x0) * fraction, y0 + (y1 - y0) * fraction));
        }
    } else {
        at_60hz.extend(clean.iter().map(|(x, y, _)| (*x, *y)));
    }
    if at_60hz.len() < 2 {
        return None;
    }

    let mut features = Array3::<f32>::zeros((1, 2, INPUT_STEPS));
    for index in 0..INPUT_STEPS {
        let position = index as f32 * (at_60hz.len() - 1) as f32 / (INPUT_STEPS - 1) as f32;
        let left = position.floor() as usize;
        let right = (left + 1).min(at_60hz.len() - 1);
        let fraction = position - left as f32;
        let x = at_60hz[left].0 + (at_60hz[right].0 - at_60hz[left].0) * fraction;
        let y = at_60hz[left].1 + (at_60hz[right].1 - at_60hz[left].1) * fraction;
        features[[0, 0, index]] = x;
        features[[0, 1, index]] = y;
    }
    Some(features)
}

fn log_add(left: f32, right: f32) -> f32 {
    if left == f32::NEG_INFINITY {
        return right;
    }
    if right == f32::NEG_INFINITY {
        return left;
    }
    let high = left.max(right);
    let low = left.min(right);
    high + (low - high).exp().ln_1p()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggest::dictionary::Dictionary;

    #[test]
    fn path_is_resampled_to_the_exported_shape() {
        let features = resample_path(&[(0.0, 0.0, 0.0), (1.0, 1.0, 1_000.0)]).expect("valid trace");
        assert_eq!(features.shape(), [1, 2, INPUT_STEPS]);
        assert!((features[[0, 0, 0]] - 0.0).abs() < f32::EPSILON);
        assert!((features[[0, 0, INPUT_STEPS - 1]] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn lexicon_index_includes_user_learned_words_without_copying_them() {
        let mut dictionary = Dictionary::new_empty();
        dictionary.record_user_word("gyatt", true);
        let mut recognizer = SwipeRecognizer::new().expect("FUTO model asset");
        recognizer.refresh_lexicon(&dictionary);
        assert!(recognizer.has_prefix(&dictionary, "gya"));
        assert!(recognizer.word_frequency(&dictionary, "gyatt").is_some());
        assert!(!recognizer.has_prefix(&dictionary, "gyatx"));
    }

    #[test]
    fn futo_encoder_and_lexicon_decode_the_published_computer_trace() {
        let letters = [
            ('a', 0.10, 0.500),
            ('b', 0.60, 0.833),
            ('c', 0.40, 0.833),
            ('d', 0.30, 0.500),
            ('e', 0.25, 0.167),
            ('f', 0.40, 0.500),
            ('g', 0.50, 0.500),
            ('h', 0.60, 0.500),
            ('i', 0.75, 0.167),
            ('j', 0.70, 0.500),
            ('k', 0.80, 0.500),
            ('l', 0.90, 0.500),
            ('m', 0.80, 0.833),
            ('n', 0.70, 0.833),
            ('o', 0.85, 0.167),
            ('p', 0.95, 0.167),
            ('q', 0.05, 0.167),
            ('r', 0.35, 0.167),
            ('s', 0.20, 0.500),
            ('t', 0.45, 0.167),
            ('u', 0.65, 0.167),
            ('v', 0.50, 0.833),
            ('w', 0.15, 0.167),
            ('x', 0.30, 0.833),
            ('y', 0.55, 0.167),
            ('z', 0.20, 0.833),
        ];
        let keys: Vec<_> = letters
            .into_iter()
            .map(|(letter, cx, cy)| SwipeKeyGeom {
                letter,
                cx,
                cy,
                hw: 0.05,
                hh: 0.10,
            })
            .collect();
        let px = [
            0.4141, 0.4478, 0.5, 0.5741, 0.6599, 0.7256, 0.7744, 0.8098, 0.8485, 0.867, 0.8737,
            0.8653, 0.8418, 0.8182, 0.8098, 0.7963, 0.7946, 0.8081, 0.8418, 0.8704, 0.9057, 0.9259,
            0.9545, 0.9697, 0.968, 0.9529, 0.9141, 0.8468, 0.7811, 0.7273, 0.6869, 0.6616, 0.6582,
            0.6431, 0.6061, 0.5572, 0.5067, 0.4663, 0.4495, 0.4461, 0.4411, 0.4192, 0.3872, 0.362,
            0.3283, 0.2795, 0.2391, 0.2323, 0.2407, 0.2593, 0.2879, 0.3249, 0.3468, 0.3569,
        ];
        let py = [
            0.8991, 0.858, 0.7876, 0.6702, 0.5352, 0.4237, 0.3357, 0.2653, 0.1655, 0.142, 0.142,
            0.2183, 0.3709, 0.588, 0.7347, 0.8462, 0.8697, 0.811, 0.6115, 0.4707, 0.3122, 0.2066,
            0.1303, 0.1068, 0.1068, 0.1068, 0.1185, 0.1596, 0.1772, 0.1772, 0.1772, 0.189, 0.189,
            0.189, 0.1831, 0.189, 0.189, 0.189, 0.189, 0.189, 0.1831, 0.1831, 0.1831, 0.1831,
            0.1831, 0.1948, 0.189, 0.1948, 0.189, 0.189, 0.189, 0.1831, 0.1831, 0.1831,
        ];
        let pt = [
            0.0, 100.0, 149.0, 197.0, 246.0, 297.0, 348.0, 399.0, 449.0, 498.0, 548.0, 598.0,
            648.0, 698.0, 749.0, 799.0, 849.0, 949.0, 999.0, 1047.0, 1100.0, 1152.0, 1197.0,
            1248.0, 1314.0, 1364.0, 1414.0, 1465.0, 1515.0, 1565.0, 1614.0, 1666.0, 1715.0, 1851.0,
            1898.0, 1951.0, 1998.0, 2049.0, 2097.0, 2165.0, 2231.0, 2279.0, 2331.0, 2382.0, 2431.0,
            2481.0, 2532.0, 2584.0, 2649.0, 2700.0, 2751.0, 2798.0, 2848.0, 2899.0,
        ];
        let trace: Vec<_> = px
            .into_iter()
            .zip(py)
            .zip(pt)
            .map(|((x, y), time)| (x, y, time))
            .collect();
        let dictionary = Dictionary::new_empty();
        let mut recognizer = SwipeRecognizer::new().expect("FUTO model asset");
        recognizer.prepare_layout(&keys);
        let candidates = recognizer.decode(&trace, &dictionary, 0.333, 6);
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("computer"),
            "{candidates:?}"
        );
    }
}
