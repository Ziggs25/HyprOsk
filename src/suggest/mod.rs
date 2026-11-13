pub mod compiler;
pub mod dictionary;
pub mod engine;
#[cfg(feature = "futo-swipe")]
#[path = "swipe_futo.rs"]
pub mod swipe;
#[cfg(not(feature = "futo-swipe"))]
pub mod swipe;
pub mod user_dict;

pub use dictionary::Dictionary;
pub use engine::SuggestEngine;
pub use user_dict::UserDictionary;
