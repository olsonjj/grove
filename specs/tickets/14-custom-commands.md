# 14: Custom commands

**What to build:** In Editor settings the user adds custom commands: a label and a template with `{path}`, plus a Run in terminal checkbox (on by default). On, Grove opens a new tab in the default terminal at the worktree and runs the command (e.g. `nvim {path}`). Off, Grove runs it in the background with the login-shell PATH (e.g. `subl {path}`). Custom commands appear in the editor ▾ menus and can be the default editor.

**Blocked by:** 13

**Status:** ready-for-agent

- [x] Add, edit, remove custom commands in settings
- [x] `{path}` substituted shell-quoted
- [x] Run in terminal on → terminal tab at worktree runs the command
- [x] Run in terminal off → background spawn with resolved PATH
- [x] Listed in ▾ menus; selectable as default editor
- [x] Failures reported in status line
- [x] Launch planning table tests: on/off, paths with spaces and quotes
