# 15: Accessibility and performance check

**What to build:** A verification pass over the finished app against the spec's non-functional requirements, fixing what fails. Produces a manual checklist kept in the repo for the platform pieces that aren't unit tested.

**Blocked by:** 07, 08, 10, 11, 13

**Status:** ready-for-agent

- [x] Every action reachable by keyboard on every surface
- [x] All icon-only buttons have accessible labels
- [x] Text meets 4.5:1 contrast in the dark theme
- [x] No state shown by color alone
- [x] Switcher appears within 100 ms of ⌥⌘K
- [x] Idle CPU near zero with all repos watched
- [ ] Manual checklist written and passed: hotkeys from other apps, popover placement on multiple displays (secondary left/right, mixed DPI), popover refresh-on-show, login item, Automation fallback, no network traffic beyond fetch — written (`specs/manual-checklist.md`); hardware items pending human run
