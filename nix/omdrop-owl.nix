# services.omdrop-owl, the module of omdrop-owl 0.3.0 and older, which
# services.owly replaced: its options carry over, with a warning for each.
{ config, lib, ... }:

let
  cfg = config.services.omdrop-owl;
  moved = from: to: "services.omdrop-owl.${from} is now services.owly.${to}.";
in
{
  imports = [
    (lib.mkRenamedOptionModule [ "services" "omdrop-owl" "enable" ] [ "services" "owly" "enable" ])
    (lib.mkRenamedOptionModule
      [ "services" "omdrop-owl" "openFirewall" ]
      [ "services" "owly" "openFirewall" ]
    )
    (lib.mkRemovedOptionModule [ "services" "omdrop-owl" "package" ]
      "services.owly.package is Owly's package, which replaced omdrop-owl's."
    )
  ];

  options.services.omdrop-owl = {
    interface = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      visible = false;
      description = "Replaced by services.owly.settings.station.";
    };
    channels = lib.mkOption {
      type = lib.types.listOf lib.types.int;
      default = [ ];
      visible = false;
      description = "Replaced by services.owly.settings.channels.";
    };
    allowUntested = lib.mkOption {
      type = lib.types.bool;
      default = false;
      visible = false;
      description = "Replaced by services.owly.settings.allow_untested.";
    };
  };

  config = {
    services.owly.settings = lib.mkMerge [
      (lib.mkIf (cfg.interface != null) { station = cfg.interface; })
      (lib.mkIf (cfg.channels != [ ]) { inherit (cfg) channels; })
      (lib.mkIf cfg.allowUntested { allow_untested = true; })
    ];
    warnings =
      lib.optional (cfg.interface != null) (moved "interface" "settings.station")
      ++ lib.optional (cfg.channels != [ ]) (moved "channels" "settings.channels")
      ++ lib.optional cfg.allowUntested (moved "allowUntested" "settings.allow_untested");
  };
}
