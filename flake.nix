{
  description = "Owly: AirDrop's Wi-Fi radio on Linux, for cards without Apple's Broadcom firmware";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      forAll = f: nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" ] (s: f nixpkgs.legacyPackages.${s});
    in
    {
      packages = forAll (pkgs: rec {
        owly = pkgs.callPackage ./nix/package.nix { };
        default = owly;
      });

      # Built from the system's nixpkgs, not this flake's, so owlyd runs the
      # system's nmcli (nix/module.nix).
      nixosModules.default = ./nix/module.nix;

      checks = forAll (pkgs: {
        owly = self.packages.${pkgs.stdenv.hostPlatform.system}.owly;
        # The module evaluates, and its unit and D-Bus files are in place.
        module =
          (nixpkgs.lib.nixosSystem {
            system = pkgs.stdenv.hostPlatform.system;
            modules = [
              self.nixosModules.default
              {
                services.owly.enable = true;
                services.owly.settings.allow_untested = true;
                networking.networkmanager.enable = true;
                boot.loader.grub.enable = false;
                fileSystems."/" = {
                  device = "none";
                  fsType = "tmpfs";
                };
                system.stateVersion = "26.05";
              }
            ];
          }).config.system.build.toplevel;
      });
    };
}
