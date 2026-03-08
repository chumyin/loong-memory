use std::collections::HashSet;

pub(crate) fn tokenize_terms(text: &str, max_terms: usize) -> Vec<String> {
    if max_terms == 0 {
        return Vec::new();
    }

    let mut tokens = Vec::new();
    let mut cjk_chars = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if is_word_char(ch) {
            current.push(ch.to_ascii_lowercase());
            continue;
        }

        if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }

        if is_cjk(ch) {
            cjk_chars.push(ch);
            tokens.push(ch.to_string());
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    for window in cjk_chars.windows(2) {
        tokens.push(format!("{}{}", window[0], window[1]));
    }

    if tokens.is_empty() {
        let fallback = text.trim().to_lowercase();
        if !fallback.is_empty() {
            tokens.push(fallback);
        }
    }

    dedupe_preserve_order(tokens, max_terms)
}

fn dedupe_preserve_order(tokens: Vec<String>, max_terms: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for token in tokens {
        if token.is_empty() {
            continue;
        }
        if seen.insert(token.clone()) {
            out.push(token);
        }
        if out.len() >= max_terms {
            break;
        }
    }
    out
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2F800..=0x2FA1F
    )
}

#[cfg(test)]
mod tests {
    use super::tokenize_terms;

    #[test]
    fn tokenizes_ascii_words() {
        let terms = tokenize_terms("Rust memory-engine v2", 16);
        assert_eq!(terms, vec!["rust", "memory-engine", "v2"]);
    }

    #[test]
    fn tokenizes_cjk_with_bigrams() {
        let terms = tokenize_terms("内存检索", 16);
        assert!(terms.contains(&"内".to_string()));
        assert!(terms.contains(&"存".to_string()));
        assert!(terms.contains(&"检".to_string()));
        assert!(terms.contains(&"索".to_string()));
        assert!(terms.contains(&"内存".to_string()));
        assert!(terms.contains(&"存检".to_string()));
        assert!(terms.contains(&"检索".to_string()));
    }
}
