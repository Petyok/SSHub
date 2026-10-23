{
  description = "SSHub: TUI SSH host launcher, tunnels and key manager";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs:
        let
          inherit (pkgs) lib;
          sshub = pkgs.rustPlatform.buildRustPackage {
            pname = "sshub";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            src = lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # System OpenSSL instead of the vendored one (see Cargo.toml `vendored`).
            buildNoDefaultFeatures = true;

            nativeBuildInputs = [ pkgs.pkg-config pkgs.installShellFiles pkgs.makeWrapper ];
            buildInputs = [ pkgs.openssl pkgs.dbus ];

            # CI already runs the suite with sshd and Graphviz available.
            doCheck = false;

            postInstall = ''
              installManPage man/sshub.1
              install -Dm644 assets/sshub.svg -t $out/share/icons/hicolor/scalable/apps
              # The template's @TERM@ launcher is for `just install`; here the
              # desktop picks the terminal.
              mkdir -p $out/share/applications
              substitute assets/sshub.desktop $out/share/applications/sshub.desktop \
                --replace-fail 'Exec=@TERM@ --class sshub -e @BIN@' 'Exec=sshub' \
                --replace-fail 'Terminal=false' 'Terminal=true'
              # ssh, ssh-keygen and ssh-add run at runtime; suffix so the user's own ssh wins.
              wrapProgram $out/bin/sshub --suffix PATH : ${lib.makeBinPath [ pkgs.openssh ]}
            '';

            meta = {
              description = "TUI SSH host launcher, tunnels and key manager";
              homepage = "https://github.com/Petyok/SSHub";
              license = lib.licenses.agpl3Plus;
              mainProgram = "sshub";
              platforms = systems;
            };
          };
        in
        {
          inherit sshub;
          default = sshub;
        });
    };
}
