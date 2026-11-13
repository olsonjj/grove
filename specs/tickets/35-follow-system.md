# 35: Follow System with live appearance tracking

**What to build:** The picker's SYSTEM group gains "Follow System" as a persisted choice. It resolves live to the mode default — dark → Grove Dark, light → Default Light — and tracks macOS appearance changes in real time: every visible window (and the settings window) re-skins when the Mac switches appearance, exactly like a picked theme. With a named theme chosen, appearance changes do nothing. Cancel behavior stays honest: the settings window captured the persisted *choice*, so if the system switches appearance while Settings is open, Cancel reverts to the original choice rather than pinning the name the system resolved to at that moment. Tests cover resolution, live re-resolution on appearance change, and no-op under a named theme.

**Blocked by:** 34

**Status:** ready-for-agent

- [ ] "Follow System" is a persisted choice, shown and checked in the SYSTEM group, with save/cancel semantics identical to a picked theme
- [ ] Follow System resolves dark → Grove Dark, light → Default Light
- [ ] macOS appearance change rethemes all visible windows live while Follow System is active — including while Settings is open
- [ ] With a named theme chosen, appearance changes are a no-op
- [ ] Cancel/Escape after an appearance change re-applies the entry choice, not the transient resolution
- [ ] Tests: resolution per mode, live re-resolution on appearance change, no-op under a named theme, cancel-after-change
- [ ] Visual check via the `qa-shots/` harness: same launch captured under dark and (forced) light appearance with Follow System chosen
- [ ] `just check` green

**Note from ticket 33 (review):** the persisted `theme` field is already live in settings files as a bare registry-name string (ticket 33 shipped it that way to avoid speculative shape), so the `ThemeChoice` reshape here must ship a serde migration (legacy bare string → `ThemeChoice::Name`) or old files stop loading. Also, `Theme::apply_config` replaces the `dark_theme`/`light_theme` slot with whatever it applies, so resolution must read the mode defaults from the `ThemeRegistry` (reinstalling Grove Dark for dark), not from the slot the last apply left behind.
