//! Quick switcher search: fuzzy match over every worktree's branch, folder and repo name.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::badge::Badge;
use crate::snapshot::{Presence, Snapshot};

/// Any contiguous run beats any scattered match.
const SUBSTRING: i32 = 1000;
const WORD_START: i32 = 200;
const FIELD_START: i32 = 10;
const WHOLE_FIELD: i32 = 300;

/// One worktree in the results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Absolute worktree path.
    pub path: PathBuf,
    /// The repo's display name.
    pub repo: String,
    /// `None` when detached.
    pub branch: Option<String>,
    /// The worktree folder's name.
    pub folder: String,
    /// Whether the worktree folder can be opened.
    pub presence: Presence,
    /// State badges shown with the result.
    pub badges: Vec<Badge>,
    /// In the recents list.
    pub recent: bool,
    /// Highlight ranges for the result's text fields.
    pub matches: Matches,
}

/// Matched byte ranges in each field, sorted and non-overlapping, for highlighting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Matches {
    /// Ranges in the branch name.
    pub branch: Vec<Range<usize>>,
    /// Ranges in the folder name.
    pub folder: Vec<Range<usize>>,
    /// Ranges in the repository display name.
    pub repo: Vec<Range<usize>>,
}

/// Worktrees whose branch, folder or repo name matches every word of `query`, best first;
/// ties go to the more recently opened, then snapshot order. An empty query lists `recents`
/// (most recent first), then every other worktree in snapshot order.
pub fn search(snapshot: &Snapshot, recents: &[PathBuf], query: &str) -> Vec<Hit> {
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut scored: Vec<(i32, usize, Hit)> = snapshot
        .repos
        .iter()
        .flat_map(|repo| repo.worktrees.iter().map(move |wt| (repo, wt)))
        .filter_map(|(repo, wt)| {
            // Scored on borrowed fields: only matches pay for a `Hit`.
            let folder = folder_name(&wt.path);
            let mut matches = Matches::default();
            let score = words
                .iter()
                .map(|w| matches.add_best(w, wt.branch.as_deref(), &folder, &repo.display_name))
                .sum::<Option<i32>>()?;
            matches.normalize();
            let rank = recents.iter().position(|p| *p == wt.path);
            let hit = Hit {
                path: wt.path.clone(),
                repo: repo.display_name.clone(),
                branch: wt.branch.clone(),
                folder: folder.into_owned(),
                presence: wt.presence,
                badges: repo.badges(wt),
                recent: rank.is_some(),
                matches,
            };
            Some((score, rank.unwrap_or(usize::MAX), hit))
        })
        .collect();
    scored.sort_by_key(|(score, rank, _)| (Reverse(*score), *rank));
    scored.into_iter().map(|(_, _, hit)| hit).collect()
}

fn folder_name(path: &Path) -> Cow<'_, str> {
    path.file_name().map_or_else(
        || Cow::Owned(path.display().to_string()),
        |n| n.to_string_lossy(),
    )
}

impl Matches {
    /// Scores `word` against its best field (branch, then folder, then repo if close) and
    /// records where it matched; `None` if no field matches.
    fn add_best(
        &mut self,
        word: &str,
        branch: Option<&str>,
        folder: &str,
        repo: &str,
    ) -> Option<i32> {
        let fields = [(branch, 30), (Some(folder), 15), (Some(repo), 0)];
        let (score, field, ranges) = fields
            .into_iter()
            .enumerate()
            .filter_map(|(i, (text, bonus))| {
                let (score, ranges) = fuzzy(word, text?)?;
                Some((score + bonus, i, ranges))
            })
            .max_by_key(|(score, ..)| *score)?;
        [&mut self.branch, &mut self.folder, &mut self.repo][field].extend(ranges);
        Some(score)
    }

    fn normalize(&mut self) {
        for ranges in [&mut self.branch, &mut self.folder, &mut self.repo] {
            ranges.sort_by_key(|r| r.start);
            let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
            for r in ranges.drain(..) {
                match merged.last_mut() {
                    Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                    _ => merged.push(r),
                }
            }
            *ranges = merged;
        }
    }
}

/// Case-insensitive match of `word` in `text`: the best contiguous run if there is one,
/// else the leftmost scattered (subsequence) match. Returns its score and byte ranges.
fn fuzzy(word: &str, text: &str) -> Option<(i32, Vec<Range<usize>>)> {
    let hay: Vec<(usize, char)> = text.char_indices().collect();
    let needle: Vec<char> = word.chars().collect();
    let n = needle.len();
    let byte_end = |i: usize| hay.get(i).map_or(text.len(), |(b, _)| *b);
    let byte_range = |from: usize, to: usize| hay[from].0..byte_end(to);

    let run = (0..(hay.len() + 1).saturating_sub(n))
        .filter(|&i| (0..n).all(|k| same(hay[i + k].1, needle[k])))
        .map(|i| {
            let mut score = SUBSTRING + 4 * n as i32;
            if word_start(&hay, i) {
                score += WORD_START;
            }
            if i == 0 {
                score += FIELD_START;
            }
            if n == hay.len() {
                score += WHOLE_FIELD;
            }
            (score, i)
        })
        .max_by_key(|&(score, i)| (score, Reverse(i)));
    if let Some((score, i)) = run {
        return Some((score, vec![byte_range(i, i + n)]));
    }

    let mut at = Vec::with_capacity(n);
    let mut from = 0;
    for &c in &needle {
        let i = (from..hay.len()).find(|&i| same(hay[i].1, c))?;
        at.push(i);
        from = i + 1;
    }
    let starts = at.iter().filter(|&&i| word_start(&hay, i)).count() as i32;
    let adjacent = at.windows(2).filter(|w| w[1] == w[0] + 1).count() as i32;
    let gaps = (at[n - 1] - at[0] + 1 - n) as i32;
    // Gaps weigh more than extra word starts, so "mn" prefers `main` to `a-m-a-i-n`.
    let field_start = if at[0] == 0 { 4 } else { 0 };
    let score = (2 * n as i32 + 4 * starts + 3 * adjacent + field_start - 2 * gaps).max(1);
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for &i in &at {
        match ranges.last_mut() {
            Some(last) if last.end == hay[i].0 => last.end = byte_end(i + 1),
            _ => ranges.push(byte_range(i, i + 1)),
        }
    }
    Some((score, ranges))
}

fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Start of the text, after a separator, or a camelCase hump.
fn word_start(hay: &[(usize, char)], i: usize) -> bool {
    let Some(prev) = i.checked_sub(1).map(|p| hay[p].1) else {
        return true;
    };
    matches!(prev, '/' | '-' | '_' | '.' | ' ') || (prev.is_lowercase() && hay[i].1.is_uppercase())
}
