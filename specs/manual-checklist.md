# Manual checklist

Platform behavior the unit tests can't reach. Run against a release build
(`cargo build --release`, or `just install`) with a scratch settings file:
`GROVE_SETTINGS_PATH=/tmp/grove-settings.json target/release/grove`.

Status: **[x]** = verified on 2026-10-07 (MacBook Pro + external 6K display at 2x, macOS 26.6,
"Displays have separate Spaces" off) · **[ ] human** = needs a person on real hardware.

## Hotkeys from other apps

- [x] ⌥⌘K with another app frontmost opens the switcher on top. Verified with System Events
  keystrokes (synthetic, not physical keys).
- [x] Esc closes it. Verified the same way.
- [ ] human: Esc gives focus back to the app that was frontmost (Finder, Safari, a full-screen
  app, another Space).
- [x] Hotkey to first frame is under 100 ms. The log line
  `[grove] switcher: hotkey → first frame N ms` read 27.6, 18.9, 20.7, 18.6, 22.0 and 21.4 ms
  across six opens, with 43 repos.
- [ ] human: ⌥⌘G from another app brings the main window forward, and reopens it if closed.
- [ ] human: both hotkeys work right after login, while Grove is in the background.

## Popover placement

- [x] On the main display, clicking the status item opens the popover centered under it. The
  popover's center x of 2035 matched the item's center, and it opened just below the menu bar.
- [ ] human: with the secondary display on the left, turn on "Displays have separate Spaces"
  and click the status item in the secondary display's menu bar. The popover should open under
  the item on that display.
- [ ] human: the same with the secondary display on the right.
- [ ] human: mixed DPI, for example a 1x external display with the 2x built-in. The popover
  should be crisp and correctly placed on each. Not verified because both displays here are 2x.
- [ ] human: with the status item near a screen edge, the popover stays 8pt inside the screen.
  This is unit-tested in `popover::tests::placed_under_the_status_item_within_its_screen`.

## Popover refresh-on-show

- [ ] human: run `touch x` in a worktree, or `git commit` in Terminal. Then open the popover.
  The row's state should be current within a moment of opening, with no stale "clean". The code
  path is `StatusItemClicked`, which opens the popover and then calls `refresh_all`.

## Login item

- [ ] human: from `/Applications/Grove.app`, go to Settings → Launch at login → Save. Grove
  should appear in System Settings → General → Login Items, and start at the next login.
- [ ] human: turning it off and saving removes the login item.
- [ ] human: a second copy outside /Applications doesn't take the login item over. This is
  unit-tested in `login_item::tests`.

## Automation fallback

- [ ] human: set the default terminal to Terminal or iTerm2 with tabs, and deny Grove under
  Privacy & Security → Automation. Opening a worktree should open a new window in that folder,
  and the status line should say to allow Grove under Automation. The hint text is unit-tested
  in `main_window::tests::automation_denied_shows_permission_hint`.
- [ ] human: if the first Automation prompt is left unanswered, the status line shows "Still
  waiting for permission…".

## Network

- [x] No sockets while idle and after hotkey use. `lsof -i -p <pid>` and `nettop -p <pid>` both
  showed nothing. With background fetch off, Grove makes no network traffic of its own.
  `hyper` and `reqwest` are linked into the dependency graph through gpui-kit-assets, but none
  of their symbols are in the release binary.
- [ ] human: with background fetch on, or on Fetch all, the only connections should be `git`
  child processes going to the repos' remotes. Check in Little Snitch or `nettop`.

## Idle CPU

- [x] Measured with the release build, 43 repos watched and background fetch off, using
  `ps -o time` over 60 s:
  - Main window closed (status item only): 0.00 s, which is 0%.
  - Main window open and idle: 0.42 s, about 0.7%. All of it is GPUI's per-vsync display-link
    callback for a visible window, according to `sample`. Grove's own code was idle.
- [x] Audited for polling. The only timers are:
  - the 30 s "fetched N min ago" tick, while the main window is open
  - the 15-minute fetch timer
  - status-cache write debounce
  - 2–10 ms polls that run only while a git or launch child process runs
  The watcher blocks on FSEvents.

## Keyboard and accessibility

- [x] Tab reaches every control, and Enter or Space activates it. Unit-tested with `tab_to`
  (`test_support.rs`):
  - main window: `main_window::tests::tab_reaches_every_control_and_comes_back_to_the_tree`,
    `keyboard_switches_repo_and_lands_in_its_tree`, `open_with_menu_works_from_the_keyboard`
    (▾ menu: arrows, Enter, Esc; Tab closes it), `empty_fallback_menu_still_closes_on_escape`
  - popover: `popover::tests::tab_reaches_header_rows_and_footer_and_wraps`
  - settings: `settings_window::tests::every_setting_is_reachable_by_keyboard`
  - first-run setup: `setup::tests::first_run_buttons_are_reachable_by_tab` (folder step
    and Git-problem step)
  - Change folder: `setup::tests::change_folder_works_from_the_keyboard`,
    `escape_cancels_change_folder`
- [x] The switcher has no Tab stops besides its search field. It is driven by ↑↓, ↵, ⌘↵, ⌥↵
  and Esc, all covered by the `switcher::tests` key tests.
- [x] Selection isn't shown by fill alone:
  - selected sidebar repo: accent bar and bold text
  - selected tree row: accent bar
  - selected switcher result: accent bar and the ↵ hint
  - Opens in: border and bold text
- [ ] human: VoiceOver reads these with their names, and announces state:
  - tree rows (selected, expanded)
  - switcher options
  - popover rows
  - icon buttons, including disabled buttons with their reason
- [ ] human: Full Keyboard Access (System Settings → Keyboard) shows a visible focus ring on
  each control.
