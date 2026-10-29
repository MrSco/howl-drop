# owly: owlyd and the owly CLI, installed by packaging/install.sh, as the
# AUR's package is.
{
  lib,
  rustPlatform,
  hostapd,
}:

rustPlatform.buildRustPackage {
  pname = "owly";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../.cargo
      ../crates
      ../xtask
      ../packaging
      ../README.md
      ../LICENSE
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  # Where hostapd is, for owlyd's default.
  env.OWLY_HOSTAPD = "${hostapd}/bin/hostapd";

  buildPhase = ''
    runHook preBuild
    cargo build --frozen --release -p owlyd -p owly-cli
    runHook postBuild
  '';
  # The workspace's tests run in CI, some in network namespaces a Nix build
  # doesn't have.
  doCheck = false;

  installPhase = ''
    runHook preInstall
    PREFIX=$out TARGET=target \
      LICENSEDIR=$out/share/licenses/owly DOCDIR=$out/share/doc/owly \
      sh packaging/install.sh owly
    runHook postInstall
  '';

  meta = {
    description = "AirDrop's Wi-Fi radio on Linux: AWDL in userspace, the owlyd radio service, and the owly CLI";
    homepage = "https://github.com/t4t5/owly";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "owly";
  };
}
