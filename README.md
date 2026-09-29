# git-wip

Continue on another machine exactly where you left off: modified, staged and
untracked files plus the current branch. No WIP commits on your branches, works
on protected branches, and `.git` is never file-synced.

A background `git wip watch` saves each machine's working state a few seconds
after files change, as a git stash commit to `refs/wip/<host>` on a remote you
choose, and fetches the other machines' states every 30 seconds.

At your next shell prompt in the repo, the newest state from another machine is
restored, but only if this machine has no changes of its own. If the other
machine only has newer commits on your branch, the branch is fast-forwarded and
your local changes stay, like `git pull`. The replaced state is kept in
`refs/wip-backup/<host>`. Restores never happen in the background, so an open
editor cannot write stale content over them unnoticed.

## Install (Nix + home-manager)

```nix
inputs.git-wip.url = "github:breuhan/git-wip";

imports = [ inputs.git-wip.homeManagerModules.default ];
programs.git-wip.enable = true;
```

## Usage

```sh
git wip enable <remote>    # opt a repo in, on every machine
git wip status             # snapshot per host and whether a restore is pending
git wip restore --force    # take the newest state even with local changes
```

Everything not in `.gitignore` is pushed, so use a private remote. Snapshot
pushes skip git hooks, so new Git LFS files in uncommitted work are not uploaded.
Submodules are not synced: neither changes inside them nor a moved submodule
pointer. "Newest" is decided by save time, so machine clocks must be in sync.

Watcher output goes to `~/Library/Logs/git-wip.log` (macOS) or
`journalctl --user -u git-wip` (Linux).

## License

MIT
