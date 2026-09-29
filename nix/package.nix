{
  lib,
  rustPlatform,
  git,
}:
rustPlatform.buildRustPackage {
  pname = "git-wip";
  version = "0.1.0";
  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;
  nativeCheckInputs = [ git ];
  meta = {
    description = "Sync uncommitted git working state between machines via refs/wip/<host>";
    homepage = "https://github.com/breuhan/git-wip";
    license = lib.licenses.mit;
    mainProgram = "git-wip";
  };
}
