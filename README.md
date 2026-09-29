# git-wip

Continue on another machine exactly where you left off: modified, staged and
untracked files plus the current branch. No WIP commits on your branches, works
on protected branches, and `.git` is never file-synced.

A background `git wip watch` saves each machine's working state a few seconds
after files change, as a git stash commit to `refs/wip/<host>` on a remote you
choose. It also fetches every 30 seconds and restores the newest state from
another machine, but only if this machine has no changes of its own.
If the other machine only has newer commits on your branch, the branch is
fast-forwarded and your local changes stay, like `git pull`.
The replaced state is kept in `refs/wip-backup/<host>`.

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

Watcher output goes to `~/Library/Logs/git-wip.log` (macOS) or
`journalctl --user -u git-wip` (Linux).

## License

MIT
