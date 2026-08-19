use crate::layout::key::{Key, KeyAction, LayerId};

#[derive(Debug, Clone)]
pub struct KeyboardRow {
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone)]
pub struct KeyboardLayout {
    pub rows: Vec<KeyboardRow>,
    pub id: LayerId,
}

impl KeyboardLayout {
    pub fn get_layout(id: LayerId, suggestions: &[String]) -> Self {
        match id {
            LayerId::Lower => Self::heliboard_lower(suggestions),
            LayerId::Upper => Self::heliboard_upper(suggestions),
            LayerId::Numbers => Self::numbers(),
            LayerId::Symbols => Self::symbols(),
            LayerId::Nav => Self::nav(),
            LayerId::Emoji => Self::emoji(),
        }
    }

    pub fn make_suggestion_row(suggestions: &[String]) -> KeyboardRow {
        if suggestions.is_empty() {
            // Idle toolbar mode (HeliBoard style)
            KeyboardRow {
                keys: vec![
                    Key::new("📋", KeyAction::Paste).with_weight(1.0).special(),
                    Key::new("123", KeyAction::SwitchLayer(LayerId::Numbers)).with_weight(1.0).special(),
                    Key::new("Nav", KeyAction::SwitchLayer(LayerId::Nav)).with_weight(1.0).special(),
                    Key::new("☺", KeyAction::SwitchLayer(LayerId::Emoji)).with_weight(1.0).special(),
                    Key::new("▼", KeyAction::Hide).with_weight(1.0).special(),
                ],
            }
        } else {
            let mut keys = Vec::new();
            for (idx, cand) in suggestions.iter().enumerate() {
                let weight = if idx == 1 { 1.3 } else { 1.0 };
                let mut k = Key::suggestion(idx, cand).with_weight(weight);
                if idx == 1 {
                    k = k.special();
                }
                keys.push(k);
            }
            KeyboardRow { keys }
        }
    }

    pub fn heliboard_lower(suggestions: &[String]) -> Self {
        Self {
            id: LayerId::Lower,
            rows: vec![
                Self::make_suggestion_row(suggestions),
                KeyboardRow {
                    keys: vec![
                        Key::text("q").with_secondary("1"),
                        Key::text("w").with_secondary("2"),
                        Key::text("e").with_secondary("3"),
                        Key::text("r").with_secondary("4"),
                        Key::text("t").with_secondary("5"),
                        Key::text("y").with_secondary("6"),
                        Key::text("u").with_secondary("7"),
                        Key::text("i").with_secondary("8"),
                        Key::text("o").with_secondary("9"),
                        Key::text("p").with_secondary("0"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::text("a").with_secondary("@"),
                        Key::text("s").with_secondary("#"),
                        Key::text("d").with_secondary("$"),
                        Key::text("f").with_secondary("%"),
                        Key::text("g").with_secondary("&"),
                        Key::text("h").with_secondary("*"),
                        Key::text("j").with_secondary("-"),
                        Key::text("k").with_secondary("+"),
                        Key::text("l").with_secondary("="),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("⇧", KeyAction::Shift).with_weight(1.3).special(),
                        Key::text("z").with_secondary("("),
                        Key::text("x").with_secondary(")"),
                        Key::text("c").with_secondary("_"),
                        Key::text("v").with_secondary("/"),
                        Key::text("b").with_secondary(":"),
                        Key::text("n").with_secondary(";"),
                        Key::text("m").with_secondary("!"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(1.3).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("?123", KeyAction::SwitchLayer(LayerId::Numbers)).with_weight(1.2).special(),
                        Key::new("☺", KeyAction::SwitchLayer(LayerId::Emoji)).with_weight(0.9).special(),
                        Key::text(",").with_weight(0.8),
                        // HeliBoard Spacebar with swipe cursor hint
                        Key::new("‹ English ›", KeyAction::Space).with_weight(4.5),
                        Key::text(".").with_weight(0.8),
                        Key::new("⏎", KeyAction::Enter).with_weight(1.4).special(),
                    ],
                },
            ],
        }
    }

    pub fn heliboard_upper(suggestions: &[String]) -> Self {
        Self {
            id: LayerId::Upper,
            rows: vec![
                Self::make_suggestion_row(suggestions),
                KeyboardRow {
                    keys: vec![
                        Key::text("Q"), Key::text("W"), Key::text("E"), Key::text("R"),
                        Key::text("T"), Key::text("Y"), Key::text("U"), Key::text("I"),
                        Key::text("O"), Key::text("P"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::text("A"), Key::text("S"), Key::text("D"), Key::text("F"),
                        Key::text("G"), Key::text("H"), Key::text("J"), Key::text("K"),
                        Key::text("L"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("⇪", KeyAction::Shift).with_weight(1.3).special(),
                        Key::text("Z"), Key::text("X"), Key::text("C"), Key::text("V"),
                        Key::text("B"), Key::text("N"), Key::text("M"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(1.3).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("?123", KeyAction::SwitchLayer(LayerId::Numbers)).with_weight(1.2).special(),
                        Key::new("☺", KeyAction::SwitchLayer(LayerId::Emoji)).with_weight(0.9).special(),
                        Key::text(",").with_weight(0.8),
                        Key::new("‹ ENGLISH ›", KeyAction::Space).with_weight(4.5),
                        Key::text(".").with_weight(0.8),
                        Key::new("⏎", KeyAction::Enter).with_weight(1.4).special(),
                    ],
                },
            ],
        }
    }

    pub fn numbers() -> Self {
        Self {
            id: LayerId::Numbers,
            rows: vec![
                Self::make_suggestion_row(&[]),
                KeyboardRow {
                    keys: vec![
                        Key::text("1"), Key::text("2"), Key::text("3"), Key::text("4"),
                        Key::text("5"), Key::text("6"), Key::text("7"), Key::text("8"),
                        Key::text("9"), Key::text("0"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::text("@"), Key::text("#"), Key::text("$"), Key::text("%"),
                        Key::text("&"), Key::text("-"), Key::text("+"), Key::text("("),
                        Key::text(")"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("=/<", KeyAction::SwitchLayer(LayerId::Symbols)).with_weight(1.3).special(),
                        Key::text("*"), Key::text("\""), Key::text("'"), Key::text(":"),
                        Key::text(";"), Key::text("!"), Key::text("?"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(1.3).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("ABC", KeyAction::SwitchLayer(LayerId::Lower)).with_weight(1.3).special(),
                        Key::new("Nav", KeyAction::SwitchLayer(LayerId::Nav)).with_weight(0.9).special(),
                        Key::text("/").with_weight(0.8),
                        Key::new("␣", KeyAction::Space).with_weight(4.2),
                        Key::text("=").with_weight(0.8),
                        Key::new("⏎", KeyAction::Enter).with_weight(1.4).special(),
                    ],
                },
            ],
        }
    }

    pub fn symbols() -> Self {
        Self {
            id: LayerId::Symbols,
            rows: vec![
                Self::make_suggestion_row(&[]),
                KeyboardRow {
                    keys: vec![
                        Key::text("~"), Key::text("`"), Key::text("|"), Key::text("•"),
                        Key::text("√"), Key::text("π"), Key::text("÷"), Key::text("×"),
                        Key::text("¶"), Key::text("∆"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::text("£"), Key::text("€"), Key::text("¥"), Key::text("^"),
                        Key::text("°"), Key::text("="), Key::text("{"), Key::text("}"),
                        Key::text("\\"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("123", KeyAction::SwitchLayer(LayerId::Numbers)).with_weight(1.3).special(),
                        Key::text("%"), Key::text("<"), Key::text(">"), Key::text("["),
                        Key::text("]"), Key::text("¡"), Key::text("¿"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(1.3).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("ABC", KeyAction::SwitchLayer(LayerId::Lower)).with_weight(1.3).special(),
                        Key::text("_").with_weight(0.9),
                        Key::new("␣", KeyAction::Space).with_weight(5.0),
                        Key::new("⏎", KeyAction::Enter).with_weight(1.4).special(),
                    ],
                },
            ],
        }
    }

    pub fn nav() -> Self {
        Self {
            id: LayerId::Nav,
            rows: vec![
                Self::make_suggestion_row(&[]),
                KeyboardRow {
                    keys: vec![
                        Key::new("Esc", KeyAction::Escape).with_weight(1.2).special(),
                        Key::new("Tab", KeyAction::Tab).with_weight(1.2).special(),
                        Key::new("Copy", KeyAction::Copy).with_weight(1.2).special(),
                        Key::new("Paste", KeyAction::Paste).with_weight(1.2).special(),
                        Key::new("▲", KeyAction::ArrowUp).with_weight(1.2).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("◀", KeyAction::ArrowLeft).with_weight(1.2).special(),
                        Key::new("▼", KeyAction::ArrowDown).with_weight(1.2).special(),
                        Key::new("▶", KeyAction::ArrowRight).with_weight(1.2).special(),
                        Key::new("⌫", KeyAction::Backspace).with_weight(1.2).special(),
                        Key::new("⏎", KeyAction::Enter).with_weight(1.2).special(),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("ABC", KeyAction::SwitchLayer(LayerId::Lower)).with_weight(2.0).special(),
                        Key::new("123", KeyAction::SwitchLayer(LayerId::Numbers)).with_weight(2.0).special(),
                        Key::new("▼ Hide", KeyAction::Hide).with_weight(2.0).special(),
                    ],
                },
            ],
        }
    }

    pub fn emoji() -> Self {
        Self {
            id: LayerId::Emoji,
            rows: vec![
                Self::make_suggestion_row(&[]),
                KeyboardRow {
                    keys: vec![
                        Key::text("😀"), Key::text("😂"), Key::text("🤣"), Key::text("😍"),
                        Key::text("🥰"), Key::text("😎"), Key::text("🤔"), Key::text("👍"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::text("🎉"), Key::text("🔥"), Key::text("✨"), Key::text("❤️"),
                        Key::text("💯"), Key::text("🚀"), Key::text("👏"), Key::text("🙏"),
                    ],
                },
                KeyboardRow {
                    keys: vec![
                        Key::new("ABC", KeyAction::SwitchLayer(LayerId::Lower)).with_weight(1.5).special(),
                        Key::text("👀"), Key::text("😴"), Key::text("💀"), Key::text("💩"),
                        Key::new("⌫", KeyAction::Backspace).with_weight(1.5).special(),
                    ],
                },
            ],
        }
    }
}
