self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.git-wip;
  # The timer must use the user's git (config, credential helpers, ssh).
  path = "${config.home.profileDirectory}/bin:/run/current-system/sw/bin:/usr/bin:/bin";
in
{
  options.programs.git-wip = {
    enable = lib.mkEnableOption "git-wip working-state sync";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
    };
    interval = lib.mkOption {
      type = lib.types.ints.positive;
      default = 60;
      description = "Seconds between `git wip save-all` runs.";
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        home.packages = [ cfg.package ];
        programs.zsh.initContent = ''
          _git_wip() { git wip restore --no-fetch; }
          autoload -Uz add-zsh-hook && add-zsh-hook chpwd _git_wip && _git_wip
        '';
        programs.fish.interactiveShellInit = ''
          function _git_wip --on-variable PWD; git wip restore --no-fetch; end
          _git_wip
        '';
      }
      (lib.mkIf pkgs.stdenv.isLinux {
        systemd.user.services.git-wip = {
          Unit.Description = "git-wip save-all";
          Service = {
            Type = "oneshot";
            ExecStart = "${lib.getExe cfg.package} save-all";
            Environment = [ "PATH=${path}" ];
          };
        };
        systemd.user.timers.git-wip = {
          Timer = {
            OnBootSec = "1min";
            OnUnitActiveSec = "${toString cfg.interval}s";
          };
          Install.WantedBy = [ "timers.target" ];
        };
      })
      (lib.mkIf pkgs.stdenv.isDarwin {
        launchd.agents.git-wip = {
          enable = true;
          config = {
            ProgramArguments = [
              (lib.getExe cfg.package)
              "save-all"
            ];
            StartInterval = cfg.interval;
            StandardErrorPath = "${config.home.homeDirectory}/Library/Logs/git-wip.log";
            EnvironmentVariables.PATH = path;
          };
        };
      })
    ]
  );
}
