# 02: First run and repo list

**What to build:** On first launch Grove asks "Where do you keep your code?". The user picks a folder in the macOS picker (no suggestion), sees a preview of repos found with worktree counts, and presses Continue. The main window opens with a sidebar listing every repo and its worktree count. Sets up the headless core library (no GPUI dependency) and the app binary, settings persisted to Application Support, login-shell PATH resolution, and the core test harness using real git repos in temp dirs.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] Core library builds and tests with no GPUI dependency
- [x] All subprocesses use the login-shell PATH; Homebrew `git` is found when launched from Finder
- [x] Setup screen states Grove reads only this folder and it can be changed later
- [x] Continue disabled until a folder is chosen; choice saved to settings
- [x] Discovery walks 2 levels deep; does not descend into a repo
- [x] Linked worktrees inside the code directory are grouped under their repo, not listed as repos
- [x] Worktrees outside the code directory are counted for their repo
- [x] Bare repos listed
- [x] No repos found → "No Git repos found in <folder>" with Choose folder…; same screen if the saved folder is later missing or empty
- [x] `git` missing or too old → blocking message with install instructions
- [x] Core tests cover: flat repo, nested org/repo, linked worktree inside and outside code dir, bare repo, empty folder

## Notes

- Min git 2.20: covers `worktree list --porcelain` (2.7), `status --porcelain=v2` (2.11), `GIT_OPTIONAL_LOCKS` (2.15); every Xcode CLT/Homebrew git on macOS 14+ is newer.
- PATH: `$SHELL -l -c` (5 s timeout), fallback = inherited PATH + /opt/homebrew/bin, /usr/local/bin, system dirs. Under `env -i HOME=$HOME` the log shows the login-shell PATH (with /opt/homebrew/bin before /usr/bin) and git resolved through it. No Homebrew git on this machine; verified instead with a stand-in git in a dir added only via `ZDOTDIR/.zprofile`, launched under `env -i` — Grove resolved that git.
- Repo id = path relative to the code dir (`org/repo`), `.git` suffix dropped; a `repo/.bare` layout is named `repo`. Shown as the bare folder name, with the parent folder added only when two repos share a name (`acme/web`, `personal/web`); sidebar sorted by shown name.
- A linked worktree whose repo is outside the code dir is skipped (spec 9: repos outside the folder are dropped).
- Dev env vars: `GROVE_SETTINGS_PATH` (settings file), `GROVE_CODE_DIR` (open setup as if this folder was just picked), `GROVE_GIT` (git binary override, used to simulate missing/old git).
- Closing the setup window before Continue quits Grove (nothing else can bring it back). Not checked by eye.
