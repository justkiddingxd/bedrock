//! Small fuzzy matcher for the quick switcher.

/// Scores `haystack` against a lowercase `needle`. All needle characters must
/// appear in order; consecutive runs and word starts score higher, gaps and
/// long haystacks score lower. Returns `None` when there is no match.
pub fn score(needle: &str, haystack: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    let mut total = 0i64;
    let mut pos = 0usize;
    let mut prev: Option<usize> = None;
    for nc in needle.chars() {
        let i = (pos..hay.len()).find(|&i| hay[i] == nc)?;
        total += 1;
        if i == 0 || matches!(hay[i - 1], '/' | ' ' | '-' | '_' | '.') {
            total += 8;
        }
        match prev {
            Some(p) if p + 1 == i => total += 6,
            Some(p) => total -= (i - p - 1).min(10) as i64,
            None => {}
        }
        prev = Some(i);
        pos = i + 1;
    }
    Some(total - hay.len() as i64 / 10)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn ranks_sensibly() {
        assert!(score("knb", "Projects/Bedrock notes/Backlog").is_some());
        assert!(score("xyz", "Projects/Bedrock").is_none());
        let exact = score("daily", "Daily/2026-10-29").unwrap();
        let scattered = score("daily", "Do any important lazy y").unwrap();
        assert!(exact > scattered);
    }
}
