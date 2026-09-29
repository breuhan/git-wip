self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.git-wip;
  # The watcher must use the user's git (config, credential helpers, ssh).
  path = "${config.home.profileDirectory}/bin:/run/current-system/sw/bin:/usr/bin:/bin";
  env = {
    PATH = path;
    GIT_WIP_FETCH_SECS = toString cfg.fetchInterval;
  };
in
{
  options.programs.git-wip = {
    enable = lib.mkEnableOption "git-wip working-state sync";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
    };
    fetchInterval = lib.mkOption {
      type = lib.types.ints.positive;
      default = 30;
      description = "Seconds between fetching and restoring other hosts' snapshots.";
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        home.packages = [ cfg.package ];
        # Local only, so cheap enough for every prompt; shows restores done in the background.
        programs.zsh.initContent = ''
          _git_wip() { git wip restore --no-fetch; }
          autoload -Uz add-zsh-hook && add-zsh-hook precmd _git_wip
        '';
        programs.fish.interactiveShellInit = ''
          function _git_wip --on-event fish_prompt; git wip restore --no-fetch; end
        '';
      }
      (lib.mkIf pkgs.stdenv.isLinux {
        systemd.user.services.git-wip = {
          Unit.Description = "git-wip watch";
          Service = {
            ExecStart = "${lib.getExe cfg.package} watch";
            Environment = lib.mapAttrsToList (k: v: "${k}=${v}") env;
            Restart = "always";
            RestartSec = 10;
          };
          Install.WantedBy = [ "default.target" ];
        };
      })
      (lib.mkIf pkgs.stdenv.isDarwin {
        launchd.agents.git-wip = {
          enable = true;
          config = {
            ProgramArguments = [
              (lib.getExe cfg.package)
              "watch"
            ];
            KeepAlive = true;
            ThrottleInterval = 10;
            StandardErrorPath = "${config.home.homeDirectory}/Library/Logs/git-wip.log";
            EnvironmentVariables = env;
          };
        };
      })
    ]
  );
}
