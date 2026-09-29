# git-wip

Continue on another machine exactly where you left off: modified, staged and
untracked files plus the current branch. No WIP commits on your branches, works
on protected branches, and `.git` is never file-synced.

Each machine saves its working state as a git stash commit to `refs/wip/<host>`
on a remote you choose. When you `cd` into the repo on another machine, the
newest state is restored, but only if that machine has no changes of its own.
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

Everything not in `.gitignore` is pushed, so use a private remote.

## License

MIT
