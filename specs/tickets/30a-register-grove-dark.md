# 30a: Restore runtime parity — register Grove Dark as the default dark theme

**What to build:** Pulled forward from ticket 33 (found by the ticket-30 review): ticket 33's "translate Grove Dark into a registered gpui-kit theme" lands early. `theme::init` currently calls `Theme::change(ThemeMode::Dark, None, cx)`, which selects gpui-kit's built-in **shadcn-neutral "Default Dark" theme** — so every call site migrated to `theme::palette(cx)` on tickets 29–30 renders shadcn-derived colors instead of Grove's (e.g. bg `#121418`→`#090909`, link `#7db7ff`→`#f9f9f9`, warn `#f7c987`→`#ca8a04`). This ticket translates Grove's palette into the theme registry and makes it the dark default, so `cx.theme()` (and therefore `palette(cx)`) resolves to Grove's values again. No user-visible change is *added*: the app returns to its pre-feature look on all migrated surfaces. gpui-kit's native widgets (tooltips, inputs) pick up Grove's tokens where mapped — consistent with the feature's end state ("Grove Dark is today's palette as a registered default theme").

**Token mapping (canonical source: `grove_theme_colors()` in `theme.rs`'s test module):**
background=bg, popover=panel, sidebar, input=field, list=raised, group_box=card, muted=disabled, selection=selected, border, foreground=text, muted_foreground=muted, link, accent, warning=warn_bg, warning_foreground=warn_text, sidebar_foreground=worktree_text. Tokens Grove doesn't map keep the gpui-kit dark defaults (`..ThemeColor::default()` semantics), matching today's widget behavior.

**Demo (acceptance):** a runtime test proves parity: after `gpui_kit::init` + `theme::init`, `palette(cx)`'s direct tokens exactly equal the fixed helpers (Rgba equality); derived tokens are covered by the existing pure tests.

**Blocked by:** 30

**Status:** ready-for-agent

- [ ] Grove Dark is registered into the gpui-kit theme registry as the dark-mode default (translated from today's fixed palette; unmapped tokens keep dark defaults)
- [ ] `theme::init` / `Theme::change(Dark, …)` applies Grove Dark at startup
- [ ] New runtime test: direct `palette(cx)` tokens equal the fixed helpers exactly under the real init path
- [ ] Existing theme and main-window suites pass; visuals return to the pre-feature look (user validated manually)
- [ ] `just check` green

**Ticket-33 dependency note:** 33 keeps bundling the ~20 vendored themes, persisting the choice, and applying it at launch; its "Grove Dark translated/registered as default" checklist item is fulfilled here.
