# OpenSwipe swipe engine (vendored subset)

This directory vendors the Qt-free, low-level swipe decoder from OpenSwipe:

- Upstream: https://github.com/itsdestin/openswipe
- Commit: da7742dca99b56b589145945be0e8f5c3aa7543c
- License: MIT (see `LICENSE`)
- Copyright: 2026 Destin Moss <https://youcoded.ai>

Only the engine sources needed by `SwipeEngine` are included: layout and
Unicode folding, the neural model runner (unused without a model file), the
lexicon/trie beam search, the geometric fallback decoder, and the engine
facade. Qt, the neural model, and OpenSwipe dictionaries/language files are
not included.

This first slice deliberately ships no trained swipe model asset. The engine
therefore runs its geometric fallback decoder only; the neural runner is
compiled but remains inactive. HyprOsk keeps its Rust dictionary loaded and
calls `setWords()` once per letter-set change with the full base unigram
vocabulary (`Dictionary::all_words(usize::MAX)`, not the old scorer's
4,000-word slice). Learned/user words, shortcuts, and bigrams are omitted in
this first slice, so swipe recognition currently covers base unigrams only.
OpenSwipe builds its own C++ lexicon from those words, so there is no
duplicate on-disk language asset and no additional model asset, but there is
a separate in-memory index. Its memory cost must be measured against the
running binary (see `TESTING.md`).

`shim.cpp` is a small HyprOsk-specific C ABI wrapper around `SwipeEngine`.
One local patch adds `SwipeEngine::rebuildGeometry()`: upstream `setLayout()`
only replaces the layout, so the wrapper explicitly rebuilds the geometric
decoder and search after geometry changes while keeping the loaded lexicon.
This avoids reloading words on resize/layout rebuilds that preserve letters.
