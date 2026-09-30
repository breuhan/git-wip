# git-wip

Continue on another machine exactly where you left off: modified, staged and
untracked files plus the current branch. No WIP commits on your branches, works
on protected branches, and `.git` is never file-synced.

A background `git wip watch` saves each machine's working state a few seconds
after files change, as a git stash commit to `refs/wip/<host>` on a remote you
choose, and fetches the other machines' states every 30 seconds.

At your next shell prompt in the repo, the newest state from another machine is
restored, but only if this machine has no changes of its own, or the other
machine had already seen them (each snapshot records, per machine, the newest
snapshot it has taken in). If both changed independently, nothing is replaced;
the machine that notices says so, the other one may not. If the other machine
only has newer commits on your branch, the branch is fast-forwarded and your
local changes stay, like `git pull`. The replaced state is kept in
`refs/wip-backup/<host>`; if applying fails, your state is put back. Restores
never happen in the background, which makes an open editor writing stale content
over them less likely, though an unsaved buffer can still do so after a restore.

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
git wip restore --merge    # both machines changed things: combine them
git wip restore --force    # take the newest state even with local changes
```

`--merge` is a three-way merge of the two working states, like `git merge` for
uncommitted work: changes to different files are combined, deletions are
applied, and conflicting edits get the usual markers (yours first). It needs
both machines on the same branch, makes no commit, and leaves nothing staged.
Your previous state is in `refs/wip-backup/<host>`.

## Security

Everything not in `.gitignore` is pushed within seconds, so use a private
remote. Snapshot pushes skip git hooks, including secret scanners, and replaced
snapshots stay on the remote as unreachable objects until it collects garbage.

Snapshots are not signed. Whoever can push `refs/wip/*` to the remote, including
any one of your machines, decides what the others restore at their next prompt.
Only sync between machines, and through a remote, that you trust equally.

## Limits

New Git LFS files in uncommitted work are not uploaded (hooks are skipped).
Submodules are not synced: neither changes inside them nor a moved submodule
pointer. "Newest" is decided by save time, unless a snapshot records having
seen yours; keep machine clocks in sync.

Watcher output goes to `~/Library/Logs/git-wip.log` (macOS) or
`journalctl --user -u git-wip` (Linux).

## TODO

- Sign snapshots and verify them before any restore, fast-forward or merge
  (for example SSH signatures checked against an allowed-signers file), so
  that push access to `refs/wip/*` alone no longer decides what a machine
  restores. See Security.

## License

MIT
