//! One update check against the repo's GitHub releases, as pure logic over one injected
//! HTTP fetch. Grove-app is a thin shell over this crate; no network is reachable from
//! here or from tests without supplying the fetch.

use serde::Deserialize;

/// The releases endpoint the check queries; the repo lives in this one place (spec:
/// `olsonjj/grove`). If the repo ever moves, this constant moves with it.
pub const RELEASES_ENDPOINT: &str = "https://api.github.com/repos/olsonjj/grove/releases";

/// Why the injected fetch of [`RELEASES_ENDPOINT`] failed. Every failure — offline,
/// non-200, malformed JSON — maps into the same check-failed outcome rather than an error
/// path, so the detail is only what a result dialog might quote.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct UpdateFetchError {
    /// The fetch's own description of the failure (transport message, HTTP status, ...).
    pub detail: String,
}

impl UpdateFetchError {
    /// Builds an error from the fetch's own description.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

/// What one update check concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateDecision {
    /// The running version is at least as new as the latest release, or no release
    /// exists yet.
    UpToDate,
    /// A release strictly newer than the running one exists, with its version and the
    /// release page URL taken from the response.
    Available {
        /// The latest release's version, with any tag `v` prefix stripped.
        version: semver::Version,
        /// The release page URL exactly as the response carried it.
        page_url: String,
    },
    /// The endpoint could not be read or understood; no conclusion was reached.
    CheckFailed,
}

/// Runs the one update check. `running` is Grove's own version plus build number as one
/// semver string, e.g. `"0.1.0+12"`; build metadata never takes part in the comparison.
///
/// `fetch` performs the single HTTP GET of [`RELEASES_ENDPOINT`] and returns the response
/// body; nothing here touches the network without it. Any fetch failure, unparsable
/// running version, or unexpected response body degrades to [`UpdateDecision::CheckFailed`].
pub fn check_for_update<F>(running: &str, fetch: F) -> UpdateDecision
where
    F: FnOnce(&str) -> Result<String, UpdateFetchError>,
{
    // A running version the semver can't read leaves no conclusion to reach, so the
    // check degrades to failed like any other unreadable input.
    let Ok(running) = semver::Version::parse(running) else {
        return UpdateDecision::CheckFailed;
    };
    let Ok(body) = fetch(RELEASES_ENDPOINT) else {
        return UpdateDecision::CheckFailed;
    };
    decision(&running, &body)
}

/// The shape the check relies on in each entry of the GitHub releases response.
#[derive(Deserialize)]
struct ReleaseEntry {
    tag_name: String,
    html_url: String,
}

/// Compares the parsed running version against one releases response body.
fn decision(running: &semver::Version, body: &str) -> UpdateDecision {
    let Ok(releases) = serde_json::from_str::<Vec<ReleaseEntry>>(body) else {
        return UpdateDecision::CheckFailed;
    };
    match releases.first() {
        // Zero releases parse as no latest release yet, which is up-to-date, not an
        // error (early builds degrade gracefully).
        None => UpdateDecision::UpToDate,
        // GitHub lists releases newest-first, so the first entry is the latest release.
        Some(ReleaseEntry { tag_name, html_url }) => match parse_tag(tag_name) {
            // A latest release the semver parser can't read is an unexpected response.
            None => UpdateDecision::CheckFailed,
            Some(latest) if latest > *running => UpdateDecision::Available {
                version: latest,
                page_url: html_url.clone(),
            },
            // Equal and older are up-to-date; only strictly greater counts.
            Some(_) => UpdateDecision::UpToDate,
        },
    }
}

/// Parses a release tag as semver, tolerating the optional leading `v`.
fn parse_tag(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canned releases response the engine tests run against.
    const LATEST: &str = r#"[{"tag_name":"v0.2.0","html_url":"https://github.com/olsonjj/grove/releases/tag/v0.2.0"}]"#;

    /// The running version of the checks that expect a decision, not a failure.
    const RUNNING: &str = "0.1.0+3";

    /// A fetch succeeding with a canned body, asserting the endpoint it was given.
    fn serving(body: &str) -> impl FnOnce(&str) -> Result<String, UpdateFetchError> + '_ {
        move |url| {
            assert_eq!(url, RELEASES_ENDPOINT);
            Ok(body.to_string())
        }
    }

    #[test]
    fn newer_release_is_available_with_the_response_page_url() {
        let decision = check_for_update(RUNNING, serving(LATEST));
        assert_eq!(
            decision,
            UpdateDecision::Available {
                version: semver::Version::new(0, 2, 0),
                page_url: "https://github.com/olsonjj/grove/releases/tag/v0.2.0".to_string(),
            }
        );
    }

    #[test]
    fn up_to_date_when_running_is_current() {
        let release = r#"[{"tag_name":"0.1.0","html_url":"https://example.com/tag"}]"#;
        assert_eq!(
            check_for_update(RUNNING, serving(release)),
            UpdateDecision::UpToDate
        );
    }

    #[test]
    fn equal_and_older_tags_never_count_as_available() {
        for release in [
            r#"[{"tag_name":"v0.1.0","html_url":"https://example.com/tag"}]"#,
            r#"[{"tag_name":"v0.1.0-rc.1","html_url":"https://example.com/tag"}]"#,
            r#"[{"tag_name":"0.0.9","html_url":"https://example.com/tag"}]"#,
        ] {
            assert_eq!(
                check_for_update(RUNNING, serving(release)),
                UpdateDecision::UpToDate
            );
        }
    }

    #[test]
    fn build_numbers_never_take_part() {
        // Same version with a different build number is current, not a new release.
        let release = r#"[{"tag_name":"0.1.0","html_url":"https://example.com/tag"}]"#;
        assert_eq!(
            check_for_update("0.1.0+9", serving(release)),
            UpdateDecision::UpToDate
        );
        // A newer patch still counts despite the running build number.
        let newer = r#"[{"tag_name":"0.1.1","html_url":"https://example.com/tag"}]"#;
        assert!(matches!(
            check_for_update(RUNNING, serving(newer)),
            UpdateDecision::Available { .. }
        ));
    }

    #[test]
    fn zero_releases_are_up_to_date_not_an_error() {
        assert_eq!(
            check_for_update(RUNNING, serving("[]")),
            UpdateDecision::UpToDate
        );
    }

    #[test]
    fn fetch_failure_maps_to_check_failed() {
        // Offline and HTTP status are the same fetch failure the app's getter raises.
        let decision = check_for_update(RUNNING, |_| {
            Err(UpdateFetchError::new("connection refused"))
        });
        assert_eq!(decision, UpdateDecision::CheckFailed);
        let decision = check_for_update(RUNNING, |_| Err(UpdateFetchError::new("403")));
        assert_eq!(decision, UpdateDecision::CheckFailed);
    }

    #[test]
    fn malformed_responses_map_to_check_failed() {
        for body in ["not json", "{}", "null", r#"[{"tag_name":"v0.2.0"}]"#, ""] {
            assert_eq!(
                check_for_update(RUNNING, serving(body)),
                UpdateDecision::CheckFailed
            );
        }
    }

    #[test]
    fn v_tag_prefix_is_tolerated() {
        let release = r#"[{"tag_name":"v0.2.0","html_url":"https://example.com/tag"}]"#;
        assert!(matches!(
            check_for_update("0.1.0", serving(release)),
            UpdateDecision::Available { .. }
        ));
    }

    #[test]
    fn unparsable_release_tag_degrades_to_check_failed() {
        let release = r#"[{"tag_name":"nightly","html_url":"https://example.com/tag"}]"#;
        assert_eq!(
            check_for_update(RUNNING, serving(release)),
            UpdateDecision::CheckFailed
        );
    }

    #[test]
    fn unparsable_running_version_degrades_to_check_failed() {
        let decision = check_for_update("not a version", serving(LATEST));
        assert_eq!(decision, UpdateDecision::CheckFailed);
    }

    #[test]
    fn fetch_error_detail_survives_as_the_error_message() {
        let e = UpdateFetchError::new("connection refused");
        assert_eq!(e.to_string(), "connection refused");
    }
}
