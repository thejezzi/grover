//! Repo selection by fuzzy match.
//!
//! Tiers from strict to loose: exact, prefix, substring, one typo (including
//! transposition), then Jaro-Winkler. As soon as one tier has several hits the
//! selection is ambiguous and never guessed.

use crate::error::{Error, Result};

/// Similarity from which a name counts as meant.
const SIMILAR: f64 = 0.7;

/// Index of the single match for `query`.
pub fn select(names: &[&str], query: &str) -> Result<usize> {
    let needle = query.to_lowercase();
    let lower: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();

    let tiers = [
        candidates(&lower, |name| name == needle),
        candidates(&lower, |name| name.starts_with(&needle)),
        candidates(&lower, |name| name.contains(&needle)),
        candidates(&lower, |name| strsim::damerau_levenshtein(name, &needle) <= 1),
    ];
    for hits in tiers {
        match hits.len() {
            0 => continue,
            1 => return Ok(hits[0]),
            _ => return Err(ambiguous(query, names, &hits)),
        }
    }

    let mut scored: Vec<(usize, f64)> = lower
        .iter()
        .enumerate()
        .map(|(index, name)| (index, strsim::jaro_winkler(name, &needle)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));

    let best = scored.first().map_or(0.0, |&(_, score)| score);
    if best < SIMILAR {
        let suggestions: Vec<&str> =
            scored.iter().take(3).map(|&(index, _)| names[index]).collect();
        return Err(Error::UnknownRepo {
            query: query.to_string(),
            suggestions: join(&suggestions),
        });
    }

    let hits: Vec<usize> =
        scored.iter().filter(|&&(_, score)| score == best).map(|&(index, _)| index).collect();
    match hits.len() {
        1 => Ok(hits[0]),
        _ => Err(ambiguous(query, names, &hits)),
    }
}

/// Indices of the names `pred` accepts.
fn candidates(lower: &[String], pred: impl Fn(&str) -> bool) -> Vec<usize> {
    (0..lower.len()).filter(|&index| pred(&lower[index])).collect()
}

fn ambiguous(query: &str, names: &[&str], hits: &[usize]) -> Error {
    let candidates: Vec<&str> = hits.iter().map(|&index| names[index]).collect();
    Error::AmbiguousRepo { query: query.to_string(), candidates: join(&candidates) }
}

fn join(names: &[&str]) -> String {
    if names.is_empty() { "none".to_string() } else { names.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPOS: &[&str] = &["api", "api-gateway", "web", "worker"];

    #[test]
    fn exact_match_wins() {
        assert_eq!(select(REPOS, "web").ok(), Some(2));
        assert_eq!(select(REPOS, "API").ok(), Some(0));
    }

    #[test]
    fn prefix_matches_when_unique() {
        assert_eq!(select(REPOS, "wo").ok(), Some(3));
    }

    #[test]
    fn transposition_is_one_typo() {
        assert_eq!(select(REPOS, "wbe").ok(), Some(2));
    }

    #[test]
    fn ambiguity_aborts() {
        let err = select(REPOS, "ap").expect_err("ambiguous");
        assert!(err.to_string().contains("api, api-gateway"), "{err}");
    }

    #[test]
    fn unknown_names_three_similar_ones() {
        let err = select(REPOS, "database").expect_err("unknown");
        let text = err.to_string();
        assert!(text.contains("unknown repo"), "{text}");
        assert!(text.contains("similar:"), "{text}");
    }

    #[test]
    fn no_names_means_no_match() {
        assert!(select(&[], "api").is_err());
    }
}
