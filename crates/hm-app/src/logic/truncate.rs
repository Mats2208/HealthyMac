//! Truncates long paths from the middle, where the least useful part of a path lives.

const ELLIPSIS: char = '…';

/// Shortens `text` to at most `max_chars` characters by removing its middle. The tail keeps a little
/// more than the head, because the end of a path is what tells paths apart.
pub fn truncate_middle(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return ELLIPSIS.to_string();
    }
    let keep = max_chars - 1;
    let head = keep * 2 / 5;
    let tail = keep - head;
    let mut out = String::with_capacity(max_chars * 2);
    out.extend(text.chars().take(head));
    out.push(ELLIPSIS);
    out.extend(text.chars().skip(count - tail));
    out
}

/// Finds the longest middle truncation of `text` that fits `max_width`, using `measure` to obtain
/// real text widths. Costs about log2(len) measurements.
pub fn fit_middle(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= max_width {
        return text.to_string();
    }
    let mut low = 0usize;
    let mut high = text.chars().count();
    while low < high {
        let middle = (low + high).div_ceil(2);
        if measure(&truncate_middle(text, middle)) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    truncate_middle(text, low)
}

/// Finds the longest cut from the end that fits `max_width`, ending in an ellipsis.
pub fn fit_end(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> String {
    let characters: Vec<char> = text.chars().collect();
    let with_ellipsis = |keep: usize| -> String {
        characters[..keep]
            .iter()
            .chain(std::iter::once(&ELLIPSIS))
            .collect()
    };
    if measure(&with_ellipsis(0)) > max_width {
        return String::new();
    }
    let mut low = 0usize;
    let mut high = characters.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        if measure(&with_ellipsis(middle)) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    with_ellipsis(low)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(truncate_middle("/tmp", 10), "/tmp");
        assert_eq!(truncate_middle("/tmp", 4), "/tmp");
    }

    #[test]
    fn long_text_keeps_both_ends_and_respects_the_limit() {
        let path = "/Users/ana/Library/Developer/Xcode/DerivedData/MyApp-abcdef/Build";
        let out = truncate_middle(path, 24);
        assert_eq!(out.chars().count(), 24);
        assert!(out.starts_with("/Users/"));
        assert!(out.ends_with("abcdef/Build"));
        assert!(out.contains('…'));
    }

    #[test]
    fn multibyte_characters_are_never_split() {
        let path = "/Users/año/Música/Canciones de otoño/álbum número uno/pista.flac";
        for limit in 0..path.chars().count() {
            assert!(truncate_middle(path, limit).chars().count() <= limit);
        }
    }

    #[test]
    fn degenerate_limits() {
        assert_eq!(truncate_middle("abcdef", 0), "");
        assert_eq!(truncate_middle("abcdef", 1), "…");
        assert_eq!(truncate_middle("abcdef", 2), "…f");
        assert_eq!(truncate_middle("", 5), "");
    }

    #[test]
    fn fit_uses_the_measure_and_stays_within_width() {
        let measure = |text: &str| text.chars().count() as f32 * 7.0;
        let path = "/Users/ana/Library/Caches/com.example.something/very/deep/file.bin";
        let out = fit_middle(path, 140.0, measure);
        assert!(measure(&out) <= 140.0);
        assert_eq!(out.chars().count(), 20, "uses all the room there is");
        assert_eq!(fit_middle("corto", 140.0, measure), "corto");
    }

    #[test]
    fn fit_with_no_room_is_empty() {
        let measure = |text: &str| text.chars().count() as f32 * 7.0;
        assert_eq!(fit_middle("/Users/ana", 0.0, measure), "");
        assert_eq!(fit_middle("/Users/ana", -5.0, measure), "");
    }

    #[test]
    fn end_fit_cuts_at_character_boundaries_and_fits() {
        let measure = |text: &str| text.chars().count() as f32 * 7.0;
        let title = "Instantáneas locales de Time Machine";
        for width in [0.0, 6.0, 7.0, 40.0, 100.0, 200.0] {
            let out = fit_end(title, width, measure);
            assert!(measure(&out) <= width, "{out:?} at {width}");
        }
        assert_eq!(fit_end("Instantáneas", 7.0 * 5.0, measure), "Inst…");
        assert_eq!(
            fit_end("abc", 3.0, measure),
            "",
            "not even the ellipsis fits"
        );
    }
}
