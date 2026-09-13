{
  description = "Otto — a visually-focused Wayland desktop built on Smithay and Skia";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      # x86_64 only: the pinned Skia archive and meta.platforms both are.
      systems = [ "x86_64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
      pkgsFor = system: import nixpkgs { inherit system; overlays = [ self.overlays.default ]; };

      # What cargo actually compiles. Packaging, prose and specs are left out
      # so that editing the module or the docs does not invalidate the build —
      # a full Otto rebuild is twenty minutes.
      inherit (nixpkgs) lib;
      ottoSrc = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.difference ./. (lib.fileset.unions [
          ./nix
          ./docs
          ./specs
          ./.github
          ./flake.nix
          ./flake.lock
        ]);
      };
    in
    {
      overlays.default = final: prev: {
        otto = final.callPackage ./nix/package.nix { src = ottoSrc; };
      };

      packages = forAllSystems (system:
        let pkgs = pkgsFor system; in
        {
          otto = pkgs.otto;
          default = pkgs.otto;
        });

      # `programs.otto.enable = true;` — see nix/module.nix and docs/user/nixos.md.
      nixosModules.otto = { pkgs, lib, ... }: {
        imports = [ ./nix/module.nix ];
        programs.otto.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.otto;
      };
      nixosModules.default = self.nixosModules.otto;

      # A NixOS machine built from the module. Otto renders through KMS on a
      # real GPU, so this is not a way to run the desktop — pass a GPU through
      # to the guest for that. It is here to exercise the packaging.
      #   nix build .#nixosConfigurations.vm.config.system.build.vm
      #   ./result/bin/run-otto-vm-vm
      nixosConfigurations.vm = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [ self.nixosModules.otto ./nix/vm.nix ];
      };

      # `.#checks.x86_64-linux.otto` builds the package and runs its tests;
      # `.#checks.x86_64-linux.vm` also boots that machine and checks the
      # package and the module assembled it correctly.
      checks = forAllSystems (system:
        let pkgs = pkgsFor system; in
        {
          otto = pkgs.otto;
          vm = pkgs.callPackage ./nix/vm-test.nix { ottoModule = self.nixosModules.otto; };
        });

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);

      devShells = forAllSystems (system:
        let pkgs = pkgsFor system; in
        {
          default = pkgs.mkShell {
            inputsFrom = [ pkgs.otto ];
            packages = with pkgs; [ cargo rustc rustfmt clippy rust-analyzer ];
            inherit (pkgs.otto) SKIA_BINARIES_URL;
          };
        });
    };
}
