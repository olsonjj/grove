# 02: First run and repo list

**What to build:** On first launch Grove asks "Where do you keep your code?". The user picks a folder in the macOS picker (no suggestion), sees a preview of repos found with worktree counts, and presses Continue. The main window opens with a sidebar listing every repo and its worktree count. Sets up the headless core library (no GPUI dependency) and the app binary, settings persisted to Application Support, login-shell PATH resolution, and the core test harness using real git repos in temp dirs.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Core library builds and tests with no GPUI dependency
- [ ] All subprocesses use the login-shell PATH; Homebrew `git` is found when launched from Finder
- [ ] Setup screen states Grove reads only this folder and it can be changed later
- [ ] Continue disabled until a folder is chosen; choice saved to settings
- [ ] Discovery walks 2 levels deep; does not descend into a repo
- [ ] Linked worktrees inside the code directory are grouped under their repo, not listed as repos
- [ ] Worktrees outside the code directory are counted for their repo
- [ ] Bare repos listed
- [ ] No repos found → "No Git repos found in <folder>" with Choose folder…; same screen if the saved folder is later missing or empty
- [ ] `git` missing or too old → blocking message with install instructions
- [ ] Core tests cover: flat repo, nested org/repo, linked worktree inside and outside code dir, bare repo, empty folder
