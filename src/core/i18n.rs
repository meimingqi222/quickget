//! 国际化语言枚举与双语文本。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    Zh,
    En,
}

impl Language {
    pub fn code(&self) -> &'static str {
        match self {
            Language::Zh => "zh",
            Language::En => "en",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Language::Zh => "中",
            Language::En => "EN",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            Language::Zh => Language::En,
            Language::En => Language::Zh,
        }
    }

    /// 从 POSIX/BCP-47 风格的语言标记推断：`zh` 开头算中文，其余英文。
    pub fn from_locale_tag(tag: &str) -> Self {
        let head = tag
            .split(['-', '_', '.', '@', ','])
            .next()
            .unwrap_or("")
            .trim();
        if head.eq_ignore_ascii_case("zh") {
            Language::Zh
        } else {
            Language::En
        }
    }
}

/// 一条随数据一起走的双语文案。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Text {
    Same(String),
    Pair { zh: String, en: String },
}

impl Text {
    pub fn new(zh: impl Into<String>, en: impl Into<String>) -> Self {
        Text::Pair {
            zh: zh.into(),
            en: en.into(),
        }
    }

    pub fn same(s: impl Into<String>) -> Self {
        Text::Same(s.into())
    }

    pub fn get(&self, lang: Language) -> &str {
        match self {
            Text::Same(s) => s,
            Text::Pair { zh, en } => match lang {
                Language::Zh => zh,
                Language::En => en,
            },
        }
    }
}

pub fn bilingual(f: impl Fn(Language) -> String) -> Text {
    Text::Pair {
        zh: f(Language::Zh),
        en: f(Language::En),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags_map_to_a_language() {
        for zh in ["zh", "zh-CN", "zh_TW.UTF-8", "ZH-Hans"] {
            assert_eq!(Language::from_locale_tag(zh), Language::Zh, "{zh}");
        }
        for en in ["en", "en-US", "ja-JP", "de_DE", ""] {
            assert_eq!(Language::from_locale_tag(en), Language::En, "{en}");
        }
    }
}
