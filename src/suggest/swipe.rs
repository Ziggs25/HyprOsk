use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double};

#[repr(C)]
struct HpSwipeEngine {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn hyprosk_swipe_engine_new() -> *mut HpSwipeEngine;
    fn hyprosk_swipe_engine_free(handle: *mut HpSwipeEngine);
    fn hyprosk_swipe_engine_set_layout(
        handle: *mut HpSwipeEngine,
        labels: *const u32,
        geometry: *const f32,
        count: usize,
    ) -> i32;
    fn hyprosk_swipe_engine_refresh_geometry(handle: *mut HpSwipeEngine) -> i32;
    fn hyprosk_swipe_engine_set_words(
        handle: *mut HpSwipeEngine,
        words: *const *const c_char,
        freqs: *const u8,
        count: usize,
    ) -> usize;
    fn hyprosk_swipe_engine_ready(handle: *const HpSwipeEngine) -> i32;
    fn hyprosk_swipe_engine_decode(
        handle: *mut HpSwipeEngine,
        points: *const f32,
        times: *const c_double,
        count: usize,
        aspect: f32,
        top_n: i32,
        out: *mut c_char,
        out_cap: usize,
        out_len: *mut usize,
    ) -> i32;
}

/// A letter key in the OpenSwipe layout coordinate system:
/// centre and half-size normalised by the letter area's width/height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwipeKeyGeom {
    pub letter: char,
    pub cx: f32,
    pub cy: f32,
    pub hw: f32,
    pub hh: f32,
}

/// Owns the vendored OpenSwipe `SwipeEngine` and feeds it HyprOsk's already
/// loaded dictionary. The engine is Qt-free. This first slice ships no
/// trained swipe model asset, so `SwipeEngine` uses its geometric fallback
/// decoder and works from the live key geometry.
pub struct SwipeRecognizer {
    handle: *mut HpSwipeEngine,
    letters: Vec<char>,
    geometry: Vec<SwipeKeyGeom>,
    words_loaded: bool,
    words_revision: Option<u64>,
}

impl SwipeRecognizer {
    pub fn new() -> Option<Self> {
        let handle = unsafe { hyprosk_swipe_engine_new() };
        if handle.is_null() {
            return None;
        }
        Some(Self {
            handle,
            letters: Vec::new(),
            geometry: Vec::new(),
            words_loaded: false,
            words_revision: None,
        })
    }

    /// Installs the current letter geometry. Returns true when the caller
    /// must call `load_words` before decoding: on the first valid layout, and
    /// again whenever the letter set changes (or a previous load failed).
    ///
    /// Geometry-only changes (resize, custom layout, layer rebuild with the
    /// same letters) rebuild the geometric decoder in place without reloading
    /// the lexicon, so `load_words` stays a once-per-letter-set operation.
    pub fn prepare_layout(&mut self, keys: &[SwipeKeyGeom]) -> bool {
        let mut sorted = keys.to_vec();
        sorted.sort_by_key(|key| key.letter);
        sorted.dedup_by_key(|key| key.letter);
        if sorted.is_empty() {
            return false;
        }

        let letters: Vec<char> = sorted.iter().map(|key| key.letter).collect();
        let letters_changed = letters != self.letters;
        let geometry_changed = sorted != self.geometry;
        if geometry_changed {
            let labels: Vec<u32> = sorted.iter().map(|key| key.letter as u32).collect();
            let mut geometry = Vec::with_capacity(sorted.len() * 4);
            for key in &sorted {
                geometry.extend_from_slice(&[key.cx, key.cy, key.hw, key.hh]);
            }
            let ok = unsafe {
                hyprosk_swipe_engine_set_layout(
                    self.handle,
                    labels.as_ptr(),
                    geometry.as_ptr(),
                    sorted.len(),
                )
            } != 0;
            if !ok {
                self.words_loaded = false;
                return true;
            }
            self.geometry = sorted;
        }
        if letters_changed {
            self.letters = letters;
            self.words_loaded = false;
        } else if geometry_changed && self.words_loaded {
            // setLayout() only swaps the layout; the geometric decoder keeps
            // its old rectangles until rebuilt. The lexicon is unchanged.
            if unsafe { hyprosk_swipe_engine_refresh_geometry(self.handle) } == 0 {
                self.words_loaded = false;
            }
        }
        !self.words_loaded
    }

    /// Converts dictionary frequencies to OpenSwipe's 0..255 scale. A log
    /// curve keeps low-frequency words distinguishable even though HyprOsk's
    /// stored `u16` frequencies have a much wider range.
    /// True when the installed OpenSwipe lexicon is stale or missing.
    pub fn needs_dictionary_refresh(&self, revision: u64) -> bool {
        !self.words_loaded || self.words_revision != Some(revision)
    }

    pub fn load_words(&mut self, words: &[(String, u32)], revision: u64) -> bool {
        let max_freq = words.iter().map(|(_, freq)| *freq).max().unwrap_or(0);
        let denominator = (max_freq as f64).ln_1p();

        let mut strings = Vec::with_capacity(words.len());
        let mut freqs = Vec::with_capacity(words.len());
        for (word, freq) in words {
            let Ok(word) = CString::new(word.as_str()) else {
                continue;
            };
            let scaled = if denominator > 0.0 {
                (((*freq as f64).ln_1p() / denominator) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8
            } else {
                0
            };
            strings.push(word);
            freqs.push(scaled);
        }
        let pointers: Vec<*const c_char> = strings.iter().map(|word| word.as_ptr()).collect();
        let count = unsafe {
            hyprosk_swipe_engine_set_words(
                self.handle,
                pointers.as_ptr(),
                freqs.as_ptr(),
                pointers.len(),
            )
        };
        self.words_loaded = count > 0 && unsafe { hyprosk_swipe_engine_ready(self.handle) } != 0;
        if self.words_loaded {
            self.words_revision = Some(revision);
        }
        self.words_loaded
    }

    /// `points` are (x, y) normalised by the letter area's width/height,
    /// with `t` in milliseconds relative to the first point.
    pub fn decode(&mut self, points: &[(f32, f32, f64)], aspect: f32, top_n: usize) -> Vec<String> {
        if !self.words_loaded || points.is_empty() || top_n == 0 {
            return Vec::new();
        }
        let mut flat = Vec::with_capacity(points.len() * 2);
        let mut times = Vec::with_capacity(points.len());
        for &(x, y, t) in points {
            flat.push(x);
            flat.push(y);
            times.push(t);
        }
        let mut output = vec![0u8; 4096];
        let mut output_len = 0usize;
        let count = unsafe {
            hyprosk_swipe_engine_decode(
                self.handle,
                flat.as_ptr(),
                times.as_ptr(),
                points.len(),
                aspect,
                top_n.min(i32::MAX as usize) as i32,
                output.as_mut_ptr() as *mut c_char,
                output.len(),
                &mut output_len,
            )
        };
        if count <= 0 || output_len == 0 {
            return Vec::new();
        }
        let text = unsafe { CStr::from_ptr(output.as_ptr() as *const c_char) };
        text.to_str()
            .map(|text| {
                text.split('\n')
                    .filter(|word| !word.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for SwipeRecognizer {
    fn drop(&mut self) {
        unsafe { hyprosk_swipe_engine_free(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyboard() -> Vec<SwipeKeyGeom> {
        let rows = [
            ("qwertyuiop", 0.5_f32, 0.0_f32),
            ("asdfghjkl", 1.5_f32, 0.5_f32),
            ("zxcvbnm", 2.5_f32, 1.0_f32),
        ];
        let (width, height) = (10.0_f32, 3.0_f32);
        let mut keys = Vec::new();
        for (letters, y, offset) in rows {
            for (index, letter) in letters.chars().enumerate() {
                keys.push(SwipeKeyGeom {
                    letter,
                    cx: (offset + index as f32 + 0.5) / width,
                    cy: y / height,
                    hw: 0.5 / width,
                    hh: 0.5 / height,
                });
            }
        }
        keys
    }

    fn trace_for(word: &str, keys: &[SwipeKeyGeom]) -> Vec<(f32, f32, f64)> {
        let mut points = Vec::new();
        let mut time = 0.0;
        let mut previous: Option<(f32, f32)> = None;
        for letter in word.chars() {
            let key = keys
                .iter()
                .find(|key| key.letter == letter)
                .expect("letter on layout");
            let centre = (key.cx, key.cy);
            if let Some((x0, y0)) = previous {
                for step in 1..6 {
                    let fraction = step as f32 / 6.0;
                    points.push((
                        x0 + (centre.0 - x0) * fraction,
                        y0 + (centre.1 - y0) * fraction,
                        time,
                    ));
                    time += 16.0;
                }
            }
            points.push((centre.0, centre.1, time));
            time += 16.0;
            previous = Some(centre);
        }
        points
    }

    #[test]
    fn recognizes_a_clean_swipe_from_hyprosk_words() {
        let keys = keyboard();
        let mut recognizer = SwipeRecognizer::new().expect("engine");
        assert!(recognizer.prepare_layout(&keys));
        assert!(recognizer.load_words(&[
            ("hello".to_string(), 200),
            ("world".to_string(), 190),
            ("the".to_string(), 255),
        ], 0));
        assert!(!recognizer.prepare_layout(&keys), "words load only once");

        let trace = trace_for("world", &keys);
        let candidates = recognizer.decode(&trace, 0.3, 5);
        assert_eq!(candidates.first().map(String::as_str), Some("world"));
    }

    #[test]
    fn geometry_change_rebuilds_decoder_without_reloading_words() {
        let keys = keyboard();
        let mut recognizer = SwipeRecognizer::new().expect("engine");
        assert!(recognizer.prepare_layout(&keys));
        assert!(recognizer.load_words(&[
            ("hello".to_string(), 200),
            ("world".to_string(), 190),
            ("the".to_string(), 255),
        ], 0));

        let mirrored: Vec<SwipeKeyGeom> = keys
            .iter()
            .map(|key| SwipeKeyGeom {
                cx: 1.0 - key.cx,
                ..*key
            })
            .collect();
        let trace = trace_for("world", &mirrored);
        assert_ne!(
            recognizer
                .decode(&trace, 0.3, 5)
                .first()
                .map(String::as_str),
            Some("world"),
            "the old geometry must not match the mirrored trace"
        );
        assert!(
            !recognizer.prepare_layout(&mirrored),
            "same letters must not reload words"
        );
        assert_eq!(
            recognizer
                .decode(&trace, 0.3, 5)
                .first()
                .map(String::as_str),
            Some("world")
        );
    }

    #[test]
    fn learned_words_enter_the_fallback_lexicon_and_can_be_swiped() {
        let keys = keyboard();
        let mut dictionary = crate::suggest::dictionary::Dictionary::new_empty();
        dictionary.record_user_word("gyatt", true);
        dictionary.record_user_word("bruh", true);
        let learned: Vec<_> = dictionary
            .swipe_words()
            .into_iter()
            .filter(|(word, _)| word == "gyatt" || word == "bruh")
            .collect();
        assert_eq!(learned.len(), 2);

        let revision = dictionary.swipe_lexicon_revision();
        let mut recognizer = SwipeRecognizer::new().expect("engine");
        assert!(recognizer.prepare_layout(&keys));
        assert!(recognizer.load_words(&learned, revision));
        assert!(!recognizer.needs_dictionary_refresh(revision));
        assert!(recognizer.needs_dictionary_refresh(revision.wrapping_add(1)));

        for word in ["gyatt", "bruh"] {
            let candidates = recognizer.decode(&trace_for(word, &keys), 0.3, 5);
            assert!(candidates.iter().any(|candidate| candidate == word), "{word}: {candidates:?}");
        }
    }

    #[test]
    fn empty_trace_produces_no_candidates() {
        let keys = keyboard();
        let mut recognizer = SwipeRecognizer::new().expect("engine");
        assert!(recognizer.prepare_layout(&keys));
        assert!(recognizer.load_words(&[("word".to_string(), 10)], 0));
        assert!(recognizer.decode(&[], 0.3, 3).is_empty());
    }
}
