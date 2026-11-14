# 36: Core update-check engine

**What to build:** The reusable update-check engine in grove-core (`specs/updater.md`, implementation decisions "Check engine lives in grove-core" and "Version comparison"). Given the running version and one injected HTTP fetch (no real network, ever, in code or tests), one call queries the repo's GitHub Releases API, takes the latest release's tag and release page URL from the response, and produces a decision: up-to-date, available (latest version + release page URL), check-failed, or no-releases-yet (zero releases parses as up-to-date, not an error). Tag parsing tolerates an optional leading "v"; the compare is semver-based and strictly greater; build numbers never take part. All URLs come from the response — Grove constructs none. Failed requests (offline, non-200, malformed JSON) map into the check-failed outcome, never a panic or error path.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Parsing, comparison, and decision tested as plain unit tests over canned fixture responses: up-to-date, newer (available, carrying the response's release URL), check-failed, empty releases list, and "v"-tag tolerance (equal/older never count as available)
- [ ] The engine takes its HTTP getter as an injected dependency; no direct network call is reachable from tests
- [ ] Public API documented per workspace `missing_docs` / rustdoc rules
- [ ] `just check` green
